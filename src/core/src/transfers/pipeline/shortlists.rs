use chrono::NaiveDate;
use log::debug;
use rayon::prelude::*;
use std::collections::HashMap;

use crate::club::BoardTransferProposal;
use crate::club::board::{BoardDossierSummary, BoardTransferEconomics};
use crate::club::staff::StaffEventType;
use crate::club::staff::perception::PotentialEstimator;
use crate::transfers::pipeline::ScoutMonitoringStatus;
use crate::transfers::pipeline::helpers::CountryPlayerLookup;
use crate::transfers::pipeline::plausibility::{
    BuyerPlausibilityContext, TransferPlausibilityBuilder, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::{
    DetailedScoutingReport, ReportRiskFlag, ScoutingAssignment, ScoutingRecommendation,
    ShortlistCandidate, ShortlistCandidateStatus, TransferRequestStatus, TransferShortlist,
};
use crate::{Club, Country, Person, PlayerFieldPositionGroup, StaffPosition, TeamType};
use std::cmp::Ordering;

struct ShortlistResult {
    club_id: u32,
    shortlist: TransferShortlist,
    request_id: u32,
}

/// Squad depth snapshot at the main team — used to weight shortlist scoring
/// so well-covered positions don't outrank real gaps.
struct PositionDepth {
    count: usize,
    max: usize,
    best_ability: u8,
}

fn position_depth_for(club: &Club, group: PlayerFieldPositionGroup) -> Option<PositionDepth> {
    let main = club.teams.iter().find(|t| t.team_type == TeamType::Main)?;
    let max = group.ideal_squad_depth();
    let (count, best_ability) = main
        .players
        .iter()
        .filter(|p| p.position().position_group() == group)
        .map(|p| p.player_attributes.current_ability)
        .fold((0usize, 0u8), |(c, b), a| (c + 1, b.max(a)));
    Some(PositionDepth {
        count,
        max,
        best_ability,
    })
}

/// A multiplier applied to candidate score based on whether the club
/// actually needs depth at this position. Surplus positions get scaled down;
/// clear gaps get a small boost.
fn depth_weight(depth: &PositionDepth, candidate_ability: u8) -> f32 {
    if depth.count == 0 {
        return 1.25;
    }
    if depth.count >= depth.max {
        // Surplus — only candidates clearly better than our best should score well
        let gap = candidate_ability as i16 - depth.best_ability as i16;
        if gap >= 10 {
            0.95
        } else if gap >= 0 {
            0.7
        } else {
            0.4
        }
    } else {
        // Room to grow — moderate boost scaled by how thin we are
        let fill = depth.count as f32 / depth.max as f32;
        1.0 + (1.0 - fill) * 0.2
    }
}

impl PipelineProcessor {
    // ============================================================
    // Step 5: Shortlist Building
    // ============================================================

    pub fn build_shortlists(country: &mut Country, date: NaiveDate) {
        // One country walk up front so the per-report / per-listing summary
        // resolutions below are hash probes instead of country scans.
        // Rosters don't change inside this pass (results apply at the end).
        let player_lookup = CountryPlayerLookup::build(country);

        // Pass 1 (PARALLEL): each club's shortlist build reads only the
        // country, the shared lookup, and its own plan — no RNG, no
        // mutation — so the clubs fan out across the pool. Ordered
        // collect + flatten keeps the applied sequence identical to the
        // serial scan.
        let country_ref: &Country = country;
        let results: Vec<ShortlistResult> = country_ref
            .clubs
            .par_iter()
            .map(|club| Self::build_club_shortlists(country_ref, club, &player_lookup, date))
            .collect::<Vec<Vec<ShortlistResult>>>()
            .into_iter()
            .flatten()
            .collect();

        if !results.is_empty() {
            debug!("Transfer pipeline: built {} shortlists", results.len());
        }

        for result in results {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == result.club_id) {
                let plan = &mut club.transfer_plan;

                if let Some(req) = plan
                    .transfer_requests
                    .iter_mut()
                    .find(|r| r.id == result.request_id)
                {
                    req.status = TransferRequestStatus::Shortlisted;
                }

                plan.shortlists.push(result.shortlist);
            }
        }
    }

    /// One club's shortlist build — pass 1 of [`Self::build_shortlists`],
    /// hoisted per club so the scan parallelizes. Strictly read-only;
    /// the staged shortlists are applied by the caller in club order.
    fn build_club_shortlists(
        country: &Country,
        club: &Club,
        player_lookup: &CountryPlayerLookup,
        date: NaiveDate,
    ) -> Vec<ShortlistResult> {
        let mut results: Vec<ShortlistResult> = Vec::new();
        {
            let plan = &club.transfer_plan;
            let buyer_ctx = BuyerPlausibilityContext::build(country, club);

            let existing_shortlist_request_ids: Vec<u32> = plan
                .shortlists
                .iter()
                .map(|s| s.transfer_request_id)
                .collect();

            // Use assignments that have produced at least one report,
            // not just fully completed ones. This prevents the pipeline
            // from stalling when scouts can only find 1 viable candidate.
            let completed_assignments: Vec<&ScoutingAssignment> = plan
                .scouting_assignments
                .iter()
                .filter(|a| a.completed || a.reports_produced > 0)
                .collect();

            for assignment in completed_assignments {
                if existing_shortlist_request_ids.contains(&assignment.transfer_request_id) {
                    continue;
                }

                let request = plan
                    .transfer_requests
                    .iter()
                    .find(|r| r.id == assignment.transfer_request_id);
                let budget_alloc = request.map(|r| r.budget_allocation).unwrap_or(0.0);

                let reports: Vec<&DetailedScoutingReport> = plan
                    .scouting_reports
                    .iter()
                    .filter(|r| r.assignment_id == assignment.id)
                    .collect();

                if reports.is_empty() {
                    continue;
                }

                let depth = position_depth_for(club, assignment.target_position.position_group());

                let mut candidates: Vec<ShortlistCandidate> = reports
                    .iter()
                    .filter(|r| budget_alloc <= 0.0 || r.estimated_value <= budget_alloc * 2.0)
                    .filter_map(|r| {
                        // Plausibility veto — drop HardReject candidates
                        // before scoring so they never become shortlist
                        // entries. Soft Allow adjustments dampen score.
                        let summary = player_lookup.find_summary(country, r.player_id, date);
                        let plausibility = summary.as_ref().and_then(|p| {
                            TransferPlausibilityBuilder::evaluate_summary(
                                &buyer_ctx, p, false, true, date,
                            )
                        });
                        if let Some(TransferPlausibilityVerdict::HardReject(_)) = plausibility {
                            return None;
                        }
                        let plausibility_mult = plausibility
                            .map(|v| v.adjustment().shortlist_score_multiplier)
                            .unwrap_or(1.0);
                        Some((r, plausibility_mult))
                    })
                    .map(|(r, plausibility_mult)| {
                        let ability_score = r.assessed_ability as f32 / 200.0;
                        let potential_score = r.assessed_potential as f32 / 200.0;
                        let value_fit = if budget_alloc > 0.0 {
                            1.0 - (r.estimated_value / budget_alloc).min(1.0) as f32
                        } else {
                            0.5
                        };
                        let confidence_score = r.confidence;

                        let base_score = ability_score * 0.3
                            + potential_score * 0.2
                            + value_fit * 0.25
                            + confidence_score * 0.1
                            + match r.recommendation {
                                ScoutingRecommendation::StrongBuy => 0.15,
                                ScoutingRecommendation::Buy => 0.10,
                                ScoutingRecommendation::Consider => 0.05,
                                ScoutingRecommendation::Pass => 0.0,
                            };

                        // Risk flags dampen the score multiplicatively — serious
                        // flags (injured, bad attitude, wage demand) hurt more
                        // than softer ones (age, contract-expiring which is
                        // actually a mild positive).
                        let mut risk_multiplier: f32 = 1.0;
                        for flag in &r.risk_flags {
                            risk_multiplier *= match flag {
                                ReportRiskFlag::CurrentlyInjured => 0.85,
                                ReportRiskFlag::PoorAttitude => 0.8,
                                ReportRiskFlag::WageDemands => 0.75,
                                ReportRiskFlag::AgeRisk => 0.9,
                                ReportRiskFlag::ContractExpiring => 1.05,
                            };
                        }

                        let depth_mult = match &depth {
                            Some(d) => depth_weight(d, r.assessed_ability),
                            None => 1.0,
                        };
                        // role_fit is ~[0.5, 1.25]; center it around 1.0 so a perfect
                        // fit lifts score ~25% and a bad fit drops ~50%.
                        let role_mult = r.role_fit.clamp(0.5, 1.25);
                        // Meeting endorsement multiplier: candidates who
                        // were explicitly promoted by a recruitment
                        // meeting earn a small boost so they outrank
                        // raw market fallbacks at equal raw quality.
                        // Players the meeting rejected are filtered
                        // out via `rejected_players`, so the negative
                        // case is handled there.
                        let meeting_mult = if plan.scout_monitoring.iter().any(|m| {
                            m.player_id == r.player_id
                                && matches!(
                                    m.status,
                                    ScoutMonitoringStatus::PromotedToShortlist
                                        | ScoutMonitoringStatus::Negotiating
                                )
                        }) {
                            1.10
                        } else {
                            1.0
                        };
                        let score = base_score
                            * depth_mult
                            * risk_multiplier
                            * role_mult
                            * meeting_mult
                            * plausibility_mult;

                        ShortlistCandidate {
                            player_id: r.player_id,
                            score,
                            estimated_fee: r.estimated_value,
                            status: ShortlistCandidateStatus::Available,
                        }
                    })
                    .collect();

                candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
                candidates.truncate(10);

                if !candidates.is_empty() {
                    let mut shortlist =
                        TransferShortlist::new(assignment.transfer_request_id, budget_alloc);
                    shortlist.candidates = candidates;

                    results.push(ShortlistResult {
                        club_id: club.id,
                        shortlist,
                        request_id: assignment.transfer_request_id,
                    });
                }
            }

            // Market shortlist: for any request without a shortlist, also try building
            // one directly from market listings. This keeps transfers flowing while
            // scouts work on deeper evaluations in parallel.
            for request in &plan.transfer_requests {
                if request.status != TransferRequestStatus::Pending
                    && request.status != TransferRequestStatus::ScoutingActive
                {
                    continue;
                }
                // Emergency depth requests are serviced exclusively by
                // the free-agent matcher — a market shortlist would
                // point the paid negotiation path at a zero-budget
                // request.
                if request.is_emergency_free_agent_depth() {
                    continue;
                }
                if existing_shortlist_request_ids.contains(&request.id) {
                    continue;
                }
                // Skip if we already built a shortlist from scouting reports above
                if results
                    .iter()
                    .any(|r| r.club_id == club.id && r.request_id == request.id)
                {
                    continue;
                }

                let mut market_candidates: Vec<ShortlistCandidate> = country
                    .transfer_market
                    .get_available_listings()
                    .iter()
                    .filter(|l| l.club_id != club.id)
                    .filter(|l| l.is_seller_advertised())
                    .filter_map(|l| {
                        player_lookup
                            .find_summary(country, l.player_id, date)
                            .and_then(|p| {
                                // The request's age band is part of the
                                // recruitment brief — a DevelopmentSigning
                                // request must not shortlist a veteran just
                                // because he tops the group on current
                                // ability. Same relaxed band as the loan
                                // market: min strict, max + 3.
                                if p.position_group == request.position.position_group()
                                    && p.skill_ability >= request.min_ability
                                    && p.age >= request.preferred_age_min
                                    && p.age <= request.preferred_age_max.saturating_add(3)
                                    && p.estimated_value <= request.budget_allocation * 2.0
                                {
                                    // Plausibility veto: drop HardReject
                                    // entries, soft-dampen the rest.
                                    let plausibility =
                                        TransferPlausibilityBuilder::evaluate_summary(
                                            &buyer_ctx, &p, false, false, date,
                                        );
                                    if let Some(TransferPlausibilityVerdict::HardReject(_)) =
                                        plausibility
                                    {
                                        return None;
                                    }
                                    let plausibility_mult = plausibility
                                        .map(|v| v.adjustment().shortlist_score_multiplier)
                                        .unwrap_or(1.0);
                                    let rival_penalty =
                                        if club.is_rival(l.club_id) { 0.75 } else { 1.0 };
                                    let budget_fit = if request.budget_allocation > 0.0 {
                                        (1.0 - (p.estimated_value / request.budget_allocation)
                                            .min(1.2)
                                            as f32
                                            * 0.35)
                                            .clamp(0.55, 1.15)
                                    } else {
                                        0.9
                                    };
                                    Some(ShortlistCandidate {
                                        player_id: p.player_id,
                                        score: (p.skill_ability as f32 / 200.0)
                                            * budget_fit
                                            * rival_penalty
                                            * plausibility_mult,
                                        estimated_fee: p.estimated_value,
                                        status: ShortlistCandidateStatus::Available,
                                    })
                                } else {
                                    None
                                }
                            })
                    })
                    .collect();

                market_candidates
                    .sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
                market_candidates.truncate(5);

                if !market_candidates.is_empty() {
                    let mut shortlist =
                        TransferShortlist::new(request.id, request.budget_allocation);
                    shortlist.candidates = market_candidates;

                    results.push(ShortlistResult {
                        club_id: club.id,
                        shortlist,
                        request_id: request.id,
                    });
                }
            }
        }
        results
    }

    /// Board-approval pass. Runs right after shortlists are built. For
    /// each shortlist whose top candidate clearly blows past the allocated
    /// budget, or that clashes with the chairman's financial stance, the
    /// board quietly vetoes — status goes to Abandoned, chairman_loyalty
    /// drifts down, and the manager takes a job_satisfaction hit. Named
    /// targets that DO pass the filter get `board_approved = Some(true)`
    /// so the downstream negotiation pipeline can pin them as priority #1.
    /// Rough resale projection at the end of a standard deal. A buy-low /
    /// sell-high heuristic: younger players with growth headroom (potential
    /// above current ability) hold or grow their value, while ageing
    /// signings depreciate. Deliberately simple — the board only uses it to
    /// gauge whether a deal is sound for a resale-driven owner.
    fn project_resale(fee: f64, age: u8, ability: u8, potential: u8) -> f64 {
        let age_factor = if age <= 23 {
            1.15
        } else if age <= 26 {
            1.0
        } else if age <= 29 {
            0.8
        } else if age <= 32 {
            0.55
        } else {
            0.3
        };
        // Unrealised potential adds upside; a maxed-out player has none.
        let growth = ((potential as f32 - ability as f32) / 50.0).clamp(0.0, 1.0) as f64;
        fee * age_factor * (0.85 + growth * 0.4)
    }

    pub fn evaluate_board_approvals(country: &mut Country, date: NaiveDate) {
        #[derive(Clone, Copy)]
        struct PlayerApprovalSnapshot {
            age: u8,
            ability: u8,
            potential: u8,
            /// Current annual wage — the best signal we have at shortlist
            /// stage for what the player would commit the club to.
            annual_salary: u32,
            /// Nationality country id, for the homegrown / domestic check.
            country_id: u32,
            injury_proneness: u8,
            world_reputation: i16,
        }
        struct Decision {
            club_id: u32,
            request_id: u32,
            approved: bool,
            named_target: Option<u32>,
            satisfaction_delta: f32,
            loyalty_delta: i16,
            /// Lead scout id if the dossier surfaced one — used to fire
            /// a `BoardPresentation` event so the staff feed shows
            /// which scout took the dossier in front of the board.
            lead_scout_staff_id: Option<u32>,
        }
        let mut decisions: Vec<Decision> = Vec::new();
        let mut player_snapshots: HashMap<u32, PlayerApprovalSnapshot> = HashMap::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    player_snapshots.insert(
                        player.id,
                        PlayerApprovalSnapshot {
                            age: player.age(date),
                            ability: player.player_attributes.current_ability,
                            // Board approvals read the observable ceiling
                            // — hidden biological PA stays hidden.
                            potential: PotentialEstimator::observable_ceiling(player, date),
                            annual_salary: player.contract.as_ref().map(|c| c.salary).unwrap_or(0),
                            country_id: player.country_id,
                            injury_proneness: player.player_attributes.injury_proneness,
                            world_reputation: player.player_attributes.world_reputation,
                        },
                    );
                }
            }
        }

        let league_country_id = country.id;
        for club in &country.clubs {
            let plan = &club.transfer_plan;
            let remaining_transfer_budget = club
                .finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount)
                .unwrap_or(plan.total_budget);
            let squad_avg_ability = club
                .teams
                .main()
                .map(|t| t.players.current_ability_avg())
                .unwrap_or(0);

            // Wage-budget headroom for the economics dossier: the board's
            // wage mandate (season target) minus what the club already pays.
            // `None` when no target has been set yet (test fixtures / cold
            // start) — the dossier then treats wages as neutral.
            let committed_wages: f64 = club
                .teams
                .iter()
                .map(|t| t.get_annual_salary() as f64)
                .sum();
            let wage_budget = club
                .board
                .season_targets
                .as_ref()
                .map(|t| t.wage_budget.max(0) as f64);

            for shortlist in &plan.shortlists {
                // Skip anything already approved / vetoed / drained.
                let Some(top) = shortlist.candidates.first() else {
                    continue;
                };
                let req = match plan
                    .transfer_requests
                    .iter()
                    .find(|r| r.id == shortlist.transfer_request_id)
                {
                    Some(r) if r.board_approved.is_none() => r,
                    _ => continue,
                };
                if req.status != TransferRequestStatus::Shortlisted {
                    continue;
                }

                let alloc = req.budget_allocation.max(1.0);
                let fee = top.estimated_fee;

                // Veto rules live on the board so chairman temperament,
                // confidence, and long-term vision stay in one domain.
                // Pull a dossier off the recruitment-meeting state if
                // any scouts have been monitoring this candidate; the
                // board uses it to relax/tighten tolerance.
                let dossier = Self::build_board_dossier(plan, top.player_id, req.id);
                let lead_scout_id = plan
                    .scout_monitoring
                    .iter()
                    .filter(|m| m.player_id == top.player_id)
                    .max_by(|a, b| {
                        a.confidence
                            .partial_cmp(&b.confidence)
                            .unwrap_or(Ordering::Equal)
                    })
                    .map(|m| m.scout_staff_id);
                let dossier_summary = if dossier.scout_votes > 0 || dossier.matches_watched > 0 {
                    Some(BoardDossierSummary {
                        scout_votes: dossier.scout_votes,
                        chief_scout_support: dossier.chief_scout_support,
                        avg_confidence: dossier.avg_confidence,
                        avg_role_fit: dossier.avg_role_fit,
                        risk_flag_count: dossier.risk_flag_count,
                        consensus_score: dossier.consensus_score,
                        data_support: dossier.data_support,
                        matches_watched: dossier.matches_watched,
                    })
                } else {
                    None
                };
                let snapshot = player_snapshots.get(&top.player_id).copied();

                // Build the economics dossier from real data where we have
                // it, leaving genuinely unavailable signals neutral rather
                // than invented (a bad proxy is worse than a neutral 0).
                let economics = snapshot.map(|s| {
                    let wage_impact = s.annual_salary as f64;
                    // Headroom = wage mandate − committed wages. With no
                    // mandate set we leave it equal to the impact, so the
                    // board's wage-breach rule stays neutral.
                    let headroom = wage_budget
                        .map(|wb| (wb - committed_wages).max(0.0))
                        .unwrap_or(wage_impact);
                    BoardTransferEconomics {
                        wage_impact_annual: wage_impact,
                        wage_budget_headroom: headroom,
                        // Agent fee isn't modelled at this stage.
                        agent_fee: 0.0,
                        // Standard projection length; real terms are agreed
                        // later in negotiation.
                        contract_length_years: 4,
                        resale_projection: Self::project_resale(fee, s.age, s.ability, s.potential),
                        // No discipline/professionalism signal sourced yet.
                        professionalism_risk: 0.0,
                        homegrown_fit: s.country_id == league_country_id,
                        injury_risk: (s.injury_proneness as f32 / 20.0).clamp(0.0, 1.0),
                        commercial_value: (s.world_reputation as f32 / 10_000.0).clamp(0.0, 1.0),
                        manager_priority: dossier_summary.is_some(),
                    }
                });

                let proposal = BoardTransferProposal {
                    fee,
                    allocated_budget: alloc,
                    remaining_transfer_budget,
                    priority: req.priority.clone(),
                    reason: req.reason.clone(),
                    player_age: snapshot.map(|s| s.age),
                    player_ability: snapshot.map(|s| s.ability),
                    squad_avg_ability,
                    shortlist_score: top.score,
                    dossier: dossier_summary,
                    economics,
                };
                let board_decision = club.board.review_transfer_proposal(&proposal);
                let veto_reason: Option<&str> = if board_decision.is_approved() {
                    None
                } else {
                    Some("board")
                };

                if let Some(_reason) = veto_reason {
                    decisions.push(Decision {
                        club_id: club.id,
                        request_id: req.id,
                        approved: false,
                        named_target: Some(top.player_id),
                        satisfaction_delta: board_decision
                            .manager_satisfaction_delta(&req.priority),
                        // Vetoing a manager's top target is a public
                        // disagreement — shifts the chairman-manager bond.
                        loyalty_delta: board_decision.loyalty_delta(&req.priority),
                        lead_scout_staff_id: lead_scout_id,
                    });
                } else {
                    // Green-lit target. Pin it so downstream can fast-track.
                    decisions.push(Decision {
                        club_id: club.id,
                        request_id: req.id,
                        approved: true,
                        named_target: Some(top.player_id),
                        satisfaction_delta: board_decision
                            .manager_satisfaction_delta(&req.priority),
                        loyalty_delta: board_decision.loyalty_delta(&req.priority),
                        lead_scout_staff_id: lead_scout_id,
                    });
                }
            }
        }

        for d in decisions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == d.club_id) {
                if let Some(req) = club
                    .transfer_plan
                    .transfer_requests
                    .iter_mut()
                    .find(|r| r.id == d.request_id)
                {
                    req.board_approved = Some(d.approved);
                    req.named_target = d.named_target;
                    if !d.approved {
                        req.status = TransferRequestStatus::Abandoned;
                    }
                }

                // Fire the manager hit/boost + loyalty drift once per board review.
                if d.loyalty_delta != 0 {
                    let cur = club.board.chairman.manager_loyalty as i16;
                    club.board.chairman.manager_loyalty =
                        (cur + d.loyalty_delta).clamp(0, 100) as u8;
                }
                if d.satisfaction_delta.abs() > 0.01 {
                    if let Some(main_team) = club.teams.main_mut() {
                        if let Some(mgr) = main_team
                            .staffs
                            .find_mut_by_position(StaffPosition::Manager)
                        {
                            mgr.job_satisfaction =
                                (mgr.job_satisfaction + d.satisfaction_delta).clamp(0.0, 100.0);
                        }
                    }
                }
                // Surface a BoardPresentation event on the lead scout
                // so the staff page reflects who took the dossier in
                // front of the board.
                if let Some(scout_id) = d.lead_scout_staff_id {
                    for team in &mut club.teams.teams {
                        if let Some(staff) = team.staffs.find_mut(scout_id) {
                            staff.add_event(StaffEventType::BoardPresentation);
                            break;
                        }
                    }
                }
                // If approved, advance the monitoring rows for the
                // signed target so subsequent ticks see Negotiating
                // status rather than PromotedToShortlist.
                if d.approved {
                    if let Some(player_id) = d.named_target {
                        club.transfer_plan.set_monitoring_status_for_player(
                            player_id,
                            ScoutMonitoringStatus::Negotiating,
                        );
                    }
                }
                // Vetoed targets fall back to monitoring — scouts
                // keep an eye but downstream pursuit halts.
                if !d.approved {
                    if let Some(player_id) = d.named_target {
                        for m in club.transfer_plan.scout_monitoring.iter_mut() {
                            if m.player_id == player_id
                                && matches!(m.status, ScoutMonitoringStatus::PromotedToShortlist)
                            {
                                m.status = ScoutMonitoringStatus::Active;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod market_fallback_age_tests {
    use crate::club::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::fullname::FullName;
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::market::{TransferListing, TransferListingType};
    use crate::transfers::pipeline::processor::PipelineProcessor;
    use crate::transfers::pipeline::{
        TransferNeedPriority, TransferNeedReason, TransferRequest, TransferRequestStatus,
    };
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, Goalkeeping, Mental,
        PersonAttributes, Physical, Player, PlayerAttributes, PlayerCollection, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills, PlayerStatusType, StaffCollection, Team,
        TeamCollection, TeamReputation, TeamType, Technical, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    /// Fixtures for the market-fallback age-band gate. The scenario pins
    /// the Radunović bug: a Serie A club with an open GK
    /// DevelopmentSigning request (age band 16-21) must not shortlist a
    /// listed 32-year-old keeper from the domestic market just because
    /// he tops the group on current ability.
    struct MarketAgeFixtures;

    impl MarketAgeFixtures {
        const BUYER_ID: u32 = 1;
        const SELLER_ID: u32 = 2;
        const VETERAN_ID: u32 = 901;
        const PROSPECT_ID: u32 = 902;

        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        /// Uniform mid-level skills so both keepers clear the request's
        /// ability floor and are plausibility-identical — the ONLY thing
        /// separating them is the birth year.
        fn skills(v: f32) -> PlayerSkills {
            PlayerSkills {
                technical: Technical {
                    corners: v,
                    crossing: v,
                    dribbling: v,
                    finishing: v,
                    first_touch: v,
                    free_kicks: v,
                    heading: v,
                    long_shots: v,
                    long_throws: v,
                    marking: v,
                    passing: v,
                    penalty_taking: v,
                    tackling: v,
                    technique: v,
                },
                mental: Mental {
                    aggression: v,
                    anticipation: v,
                    bravery: v,
                    composure: v,
                    concentration: v,
                    decisions: v,
                    determination: v,
                    flair: v,
                    leadership: v,
                    off_the_ball: v,
                    positioning: v,
                    teamwork: v,
                    vision: v,
                    work_rate: v,
                },
                physical: Physical {
                    acceleration: v,
                    agility: v,
                    balance: v,
                    jumping: v,
                    natural_fitness: v,
                    pace: v,
                    stamina: v,
                    strength: v,
                    match_readiness: v,
                },
                goalkeeping: Goalkeeping {
                    aerial_reach: v,
                    command_of_area: v,
                    communication: v,
                    eccentricity: v,
                    first_touch: v,
                    handling: v,
                    kicking: v,
                    one_on_ones: v,
                    passing: v,
                    punching: v,
                    reflexes: v,
                    rushing_out: v,
                    throwing: v,
                },
            }
        }

        fn gk(id: u32, birth_year: i32) -> Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("Keeper{id}")))
                .birth_date(Self::d(birth_year, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(Self::skills(12.0))
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Goalkeeper,
                        level: 18,
                    }],
                })
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn team(id: u32, club_id: u32, players: Vec<Player>) -> Team {
            Team::builder()
                .id(id)
                .league_id(Some(10))
                .club_id(club_id)
                .name(format!("Team {id}"))
                .slug(format!("team-{id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(5000, 5000, 5000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(id: u32, name: &str, teams: Vec<Team>) -> Club {
            Club::new(
                id,
                name.to_string(),
                Location::new(1),
                ClubFinances::new(100_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(teams),
                ClubFacilities::default(),
            )
        }

        fn league(id: u32) -> League {
            League::new(
                id,
                "L".to_string(),
                "league".to_string(),
                1,
                500,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            )
        }

        /// Country with a buyer holding one pending GK DevelopmentSigning
        /// request and a seller advertising two listed keepers who differ
        /// only in age: a 32-year-old veteran and a 19-year-old prospect.
        fn world(date: NaiveDate) -> Country {
            let buyer = Self::club(
                Self::BUYER_ID,
                "Buyer",
                vec![Self::team(11, Self::BUYER_ID, vec![Self::gk(801, 1998)])],
            );
            let mut veteran = Self::gk(Self::VETERAN_ID, 1994);
            let mut prospect = Self::gk(Self::PROSPECT_ID, 2007);
            veteran.statuses.add(date, PlayerStatusType::Lst);
            prospect.statuses.add(date, PlayerStatusType::Lst);
            let seller = Self::club(
                Self::SELLER_ID,
                "Seller",
                vec![Self::team(21, Self::SELLER_ID, vec![veteran, prospect])],
            );

            let mut country = Country::builder()
                .id(1)
                .code("IT".to_string())
                .slug("italy".to_string())
                .name("Italy".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![Self::league(10)]))
                .clubs(vec![buyer, seller])
                .build()
                .unwrap();

            // Give the buyer real spending power — the plausibility fee
            // and wage gates read these and would otherwise HardReject
            // every candidate, hiding the age-band behaviour under test.
            country.clubs[0].finance.transfer_budget = Some(CurrencyValue {
                amount: 50_000_000.0,
                currency: Currency::Usd,
            });
            country.clubs[0].finance.wage_budget = Some(CurrencyValue {
                amount: 20_000_000.0,
                currency: Currency::Usd,
            });

            country.clubs[0]
                .transfer_plan
                .transfer_requests
                .push(TransferRequest::new(
                    1,
                    PlayerPositionType::Goalkeeper,
                    TransferNeedPriority::Optional,
                    TransferNeedReason::DevelopmentSigning,
                    40,
                    70,
                    50_000_000.0,
                ));

            for player_id in [Self::VETERAN_ID, Self::PROSPECT_ID] {
                country.transfer_market.add_listing(TransferListing::new(
                    player_id,
                    Self::SELLER_ID,
                    21,
                    CurrencyValue {
                        amount: 1_000_000.0,
                        currency: Currency::Usd,
                    },
                    date,
                    TransferListingType::Transfer,
                ));
            }
            country
        }
    }

    #[test]
    fn development_signing_market_fallback_respects_age_band() {
        let date = MarketAgeFixtures::d(2026, 7, 1);
        let mut country = MarketAgeFixtures::world(date);

        PipelineProcessor::build_shortlists(&mut country, date);

        let plan = &country.clubs[0].transfer_plan;
        let shortlist = plan
            .shortlists
            .iter()
            .find(|s| s.transfer_request_id == 1)
            .expect("market fallback must still build a shortlist from the in-band prospect");

        assert!(
            shortlist
                .candidates
                .iter()
                .any(|c| c.player_id == MarketAgeFixtures::PROSPECT_ID),
            "the 19-year-old listed keeper fits the DevelopmentSigning band and must be shortlisted"
        );
        assert!(
            !shortlist
                .candidates
                .iter()
                .any(|c| c.player_id == MarketAgeFixtures::VETERAN_ID),
            "a 32-year-old must never be shortlisted against a DevelopmentSigning request \
             (age band 16-21, relaxed max +3)"
        );
        assert_eq!(
            plan.transfer_requests[0].status,
            TransferRequestStatus::Shortlisted
        );
    }
}
