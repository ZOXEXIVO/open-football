use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

use crate::club::BoardTransferProposal;
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::{
    DetailedScoutingReport, ReportRiskFlag, ScoutingAssignment, ScoutingRecommendation,
    ShortlistCandidate, ShortlistCandidateStatus, TransferRequestStatus, TransferShortlist,
};
use crate::{Club, Country, Person, PlayerFieldPositionGroup, StaffPosition, TeamType};

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
    let max = match group {
        PlayerFieldPositionGroup::Goalkeeper => 3,
        PlayerFieldPositionGroup::Defender => 8,
        PlayerFieldPositionGroup::Midfielder => 8,
        PlayerFieldPositionGroup::Forward => 6,
    };
    let abilities: Vec<u8> = main
        .players
        .iter()
        .filter(|p| p.position().position_group() == group)
        .map(|p| p.player_attributes.current_ability)
        .collect();
    Some(PositionDepth {
        count: abilities.len(),
        max,
        best_ability: abilities.iter().copied().max().unwrap_or(0),
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
        let mut results: Vec<ShortlistResult> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;

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
                    .map(|r| {
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
                        let meeting_mult = if plan
                            .scout_monitoring
                            .iter()
                            .any(|m| {
                                m.player_id == r.player_id
                                    && matches!(
                                        m.status,
                                        crate::transfers::pipeline::ScoutMonitoringStatus::PromotedToShortlist
                                            | crate::transfers::pipeline::ScoutMonitoringStatus::Negotiating
                                    )
                            })
                        {
                            1.10
                        } else {
                            1.0
                        };
                        let score =
                            base_score * depth_mult * risk_multiplier * role_mult * meeting_mult;

                        ShortlistCandidate {
                            player_id: r.player_id,
                            score,
                            estimated_fee: r.estimated_value,
                            status: ShortlistCandidateStatus::Available,
                        }
                    })
                    .collect();

                candidates.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
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
                    .filter_map(|l| {
                        Self::find_player_summary_in_country(country, l.player_id, date).and_then(
                            |p| {
                                if p.position_group == request.position.position_group()
                                    && p.skill_ability >= request.min_ability
                                    && p.estimated_value <= request.budget_allocation * 2.0
                                {
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
                                            * rival_penalty,
                                        estimated_fee: p.estimated_value,
                                        status: ShortlistCandidateStatus::Available,
                                    })
                                } else {
                                    None
                                }
                            },
                        )
                    })
                    .collect();

                market_candidates.sort_by(|a, b| {
                    b.score
                        .partial_cmp(&a.score)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
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

    /// Board-approval pass. Runs right after shortlists are built. For
    /// each shortlist whose top candidate clearly blows past the allocated
    /// budget, or that clashes with the chairman's financial stance, the
    /// board quietly vetoes — status goes to Abandoned, chairman_loyalty
    /// drifts down, and the manager takes a job_satisfaction hit. Named
    /// targets that DO pass the filter get `board_approved = Some(true)`
    /// so the downstream negotiation pipeline can pin them as priority #1.
    pub fn evaluate_board_approvals(country: &mut Country, date: NaiveDate) {
        #[derive(Clone, Copy)]
        struct PlayerApprovalSnapshot {
            age: u8,
            ability: u8,
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
                        },
                    );
                }
            }
        }

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
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|m| m.scout_staff_id);
                let dossier_summary = if dossier.scout_votes > 0 || dossier.matches_watched > 0 {
                    Some(crate::club::board::BoardDossierSummary {
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
                            staff.add_event(
                                crate::club::staff::staff::StaffEventType::BoardPresentation,
                            );
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
                            crate::transfers::pipeline::ScoutMonitoringStatus::Negotiating,
                        );
                    }
                }
                // Vetoed targets fall back to monitoring — scouts
                // keep an eye but downstream pursuit halts.
                if !d.approved {
                    if let Some(player_id) = d.named_target {
                        for m in club.transfer_plan.scout_monitoring.iter_mut() {
                            if m.player_id == player_id
                                && matches!(
                                    m.status,
                                    crate::transfers::pipeline::ScoutMonitoringStatus::PromotedToShortlist
                                )
                            {
                                m.status =
                                    crate::transfers::pipeline::ScoutMonitoringStatus::Active;
                            }
                        }
                    }
                }
            }
        }
    }
}
