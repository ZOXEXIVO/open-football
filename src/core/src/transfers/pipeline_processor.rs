use chrono::{Datelike, NaiveDate};
use log::debug;
use std::collections::HashMap;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::market::{TransferListing, TransferListingType};
use crate::transfers::pipeline::{
    ClubTransferPlan, DetailedScoutingReport, LoanOutCandidate, LoanOutReason, LoanOutStatus,
    ScoutingAssignment, ScoutingRecommendation, ShortlistCandidate, ShortlistCandidateStatus,
    TransferApproach, TransferNeedPriority, TransferNeedReason, TransferRequest,
    TransferRequestStatus, TransferShortlist,
};
use crate::transfers::staff_resolver::StaffResolver;
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::IntegerUtils;
use crate::{
    Club, ClubTransferStrategy, Country, MatchTacticType, Person, Player,
    PlayerFieldPositionGroup, PlayerPositionType, PlayerStatusType, ReputationLevel,
    TacticsSelector, TACTICS_POSITIONS,
};

/// PipelineProcessor handles all daily transfer pipeline logic.
/// Uses a two-pass borrow pattern: immutable read -> collect mutations -> mutable write.
pub struct PipelineProcessor;

// ============================================================
// Intermediate data structures for the two-pass pattern
// ============================================================

struct SquadEvaluation {
    club_id: u32,
    requests: Vec<TransferRequest>,
    loan_outs: Vec<LoanOutCandidate>,
    total_budget: f64,
    max_concurrent: u32,
}

struct ScoutAssignmentAction {
    club_id: u32,
    assignment: ScoutingAssignment,
    request_id: u32,
}

struct ScoutingObservationResult {
    club_id: u32,
    assignment_id: u32,
    player_id: u32,
    assessed_ability: u8,
    assessed_potential: u8,
    is_new: bool,
}

struct ScoutingReportResult {
    club_id: u32,
    report: DetailedScoutingReport,
    assignment_id: u32,
}

struct ShortlistResult {
    club_id: u32,
    shortlist: TransferShortlist,
    request_id: u32,
}

struct NegotiationAction {
    club_id: u32,
    player_id: u32,
    selling_club_id: u32,
    offer: crate::transfers::offer::TransferOffer,
    is_loan: bool,
    shortlist_request_id: u32,
}

// ============================================================
// Player summary for the immutable read pass
// ============================================================

#[allow(dead_code)]
struct PlayerSummary {
    player_id: u32,
    club_id: u32,
    position: PlayerPositionType,
    position_group: PlayerFieldPositionGroup,
    current_ability: u8,
    potential_ability: u8,
    age: u8,
    estimated_value: f64,
    is_listed: bool,
    is_loan_listed: bool,
}

/// Info about a player in the squad for formation-based analysis.
struct SquadPlayerInfo {
    player_id: u32,
    primary_position: PlayerPositionType,
    current_ability: u8,
    potential_ability: u8,
    age: u8,
    position_levels: HashMap<PlayerPositionType, u8>,
}

impl PipelineProcessor {
    // ============================================================
    // Step 2: Squad Evaluation - Coach-driven, formation-based
    // ============================================================

    pub fn evaluate_squads(country: &mut Country, date: NaiveDate) {
        let should_evaluate = Self::should_evaluate(date);

        // Pass 1: Collect evaluations (immutable reads)
        let mut evaluations: Vec<SquadEvaluation> = Vec::new();

        for club in &country.clubs {
            let needs_eval = should_evaluate
                || !club.transfer_plan.initialized
                || Self::all_shortlists_exhausted(&club.transfer_plan);

            if !needs_eval {
                continue;
            }

            let eval = Self::evaluate_single_club(club, date);
            evaluations.push(eval);
        }

        // Pass 2: Apply evaluations (mutable writes)
        for eval in evaluations {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == eval.club_id) {
                let plan = &mut club.transfer_plan;

                if !plan.initialized || should_evaluate {
                    plan.transfer_requests.clear();
                    plan.scouting_assignments.clear();
                    plan.scouting_reports.clear();
                    plan.shortlists.clear();
                }

                plan.total_budget = eval.total_budget;
                plan.max_concurrent_negotiations = eval.max_concurrent;
                plan.transfer_requests.extend(eval.requests);
                plan.loan_out_candidates.extend(eval.loan_outs);
                plan.last_evaluation_date = Some(date);
                plan.initialized = true;
            }
        }
    }

    fn should_evaluate(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();

        (month == 5 && day == 31)
            || (month == 6 && day == 1)
            || (month == 1 && day == 1)
    }

    fn all_shortlists_exhausted(plan: &ClubTransferPlan) -> bool {
        if plan.shortlists.is_empty() {
            return false;
        }
        plan.shortlists.iter().all(|s| s.all_exhausted())
    }

    /// Core squad evaluation: the head coach analyzes the squad based on their preferred
    /// formation and identifies tactical gaps. The approach mirrors real football:
    ///
    /// 1. Coach determines preferred formation (using TacticsSelector logic)
    /// 2. Maps formation positions to actual squad players
    /// 3. Identifies gaps (no player), weaknesses (low quality), and depth issues
    /// 4. Club reputation determines transfer strategy (buy vs loan patterns)
    /// 5. Identifies loan-out candidates based on club tier
    fn evaluate_single_club(club: &Club, date: NaiveDate) -> SquadEvaluation {
        let mut requests = Vec::new();
        let mut loan_outs = Vec::new();
        let mut next_id = club.transfer_plan.next_request_id;

        // Calculate budget
        let budget = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or_else(|| (club.finance.balance.balance.max(0) as f64) * 0.3);

        if club.teams.teams.is_empty() {
            return SquadEvaluation {
                club_id: club.id,
                requests,
                loan_outs,
                total_budget: budget,
                max_concurrent: 1,
            };
        }

        let team = &club.teams.teams[0];
        let players = &team.players.players;

        if players.is_empty() {
            return SquadEvaluation {
                club_id: club.id,
                requests,
                loan_outs,
                total_budget: budget,
                max_concurrent: 1,
            };
        }

        // Determine club reputation tier - this drives the entire transfer strategy
        let rep_level = team.reputation.level();

        // Determine max concurrent negotiations by reputation
        let max_concurrent = match rep_level {
            ReputationLevel::Elite => 4,
            ReputationLevel::Continental => 3,
            ReputationLevel::National => 2,
            _ => 1,
        };

        // Build squad info for analysis
        let squad: Vec<SquadPlayerInfo> = players
            .iter()
            .map(|p| {
                let all_pos = p.positions();
                let mut levels = HashMap::new();
                for pos in &all_pos {
                    levels.insert(*pos, p.positions.get_level(*pos));
                }
                SquadPlayerInfo {
                    player_id: p.id,
                    primary_position: p.position(),
                    current_ability: p.player_attributes.current_ability,
                    potential_ability: p.player_attributes.potential_ability,
                    age: p.age(date),
                    position_levels: levels,
                }
            })
            .collect();

        let avg_ability: u8 = if !squad.is_empty() {
            let total: u32 = squad.iter().map(|p| p.current_ability as u32).sum();
            (total / squad.len() as u32) as u8
        } else {
            50
        };

        // ──────────────────────────────────────────────────────────
        // STEP 1: Coach determines preferred formation
        // ──────────────────────────────────────────────────────────

        // Use the existing tactics if set, otherwise determine what the coach would pick
        let formation = team
            .tactics
            .as_ref()
            .map(|t| t.tactic_type)
            .unwrap_or_else(|| {
                // Determine from coach preference
                let coach = team.staffs.head_coach();
                let available: Vec<&Player> = players.iter().collect();
                if available.len() >= 11 {
                    TacticsSelector::select(team, coach).tactic_type
                } else {
                    MatchTacticType::T442
                }
            });

        // Get the 11 positions required by this formation
        let formation_positions = Self::get_formation_positions(formation);

        // ──────────────────────────────────────────────────────────
        // STEP 2: Map formation positions to squad - find gaps
        // ──────────────────────────────────────────────────────────

        // For each formation position, find the best available player
        // A position has "coverage" if at least one player can play there adequately
        let mut used_player_ids: Vec<u32> = Vec::new();
        let mut position_coverage: Vec<(PlayerPositionType, Option<u32>, u8)> = Vec::new(); // (pos, player_id, quality)

        for &formation_pos in formation_positions {
            // Find best available player for this position (not already assigned)
            let best = squad
                .iter()
                .filter(|p| !used_player_ids.contains(&p.player_id))
                .filter_map(|p| {
                    // Check if player can play this position (exact or same group)
                    let level = p.position_levels.get(&formation_pos).copied().unwrap_or(0);
                    if level > 0 {
                        return Some((p.player_id, level, p.current_ability));
                    }
                    // Same position group with reduced effectiveness
                    if p.primary_position.position_group() == formation_pos.position_group() {
                        let reduced = (p.current_ability as u16 * 7 / 10) as u8;
                        return Some((p.player_id, 1, reduced));
                    }
                    None
                })
                .max_by_key(|&(_, level, ability)| (level as u16) * 10 + ability as u16);

            match best {
                Some((pid, _level, quality)) => {
                    used_player_ids.push(pid);
                    position_coverage.push((formation_pos, Some(pid), quality));
                }
                None => {
                    position_coverage.push((formation_pos, None, 0));
                }
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 3: Generate transfer requests from gaps
        // ──────────────────────────────────────────────────────────

        let available_budget = budget * 0.9; // Keep 10% reserve
        let mut budget_used = 0.0;

        // Count how many requests we'll make to divide budget
        let formation_gaps: Vec<_> = position_coverage
            .iter()
            .filter(|(_, player, _)| player.is_none())
            .collect();

        let quality_issues: Vec<_> = position_coverage
            .iter()
            .filter(|(_, player, quality)| {
                player.is_some() && (*quality as i16) < avg_ability as i16 - 15
            })
            .collect();

        // Count positions with only one player (need depth)
        let depth_issues: Vec<_> = formation_positions
            .iter()
            .filter(|&&pos| {
                let group = pos.position_group();
                let group_count = squad
                    .iter()
                    .filter(|p| p.primary_position.position_group() == group)
                    .count();
                // Need at least some depth per position group
                let min_needed = match group {
                    PlayerFieldPositionGroup::Goalkeeper => 2,
                    PlayerFieldPositionGroup::Defender => {
                        let formation_def = formation_positions
                            .iter()
                            .filter(|p| p.is_defender())
                            .count();
                        formation_def + 2 // +2 backup
                    }
                    PlayerFieldPositionGroup::Midfielder => {
                        let formation_mid = formation_positions
                            .iter()
                            .filter(|p| p.is_midfielder())
                            .count();
                        formation_mid + 1
                    }
                    PlayerFieldPositionGroup::Forward => {
                        let formation_fwd = formation_positions
                            .iter()
                            .filter(|p| p.is_forward())
                            .count();
                        formation_fwd + 1
                    }
                };
                group_count < min_needed
            })
            .collect();

        let total_needs = formation_gaps.len() + quality_issues.len() + depth_issues.len();
        let budget_per_need = if total_needs > 0 {
            available_budget / total_needs as f64
        } else {
            0.0
        };

        // Formation gaps - CRITICAL: no one can play the position
        for (pos, _, _) in &formation_gaps {
            let alloc = (budget_per_need * 1.5).min(available_budget - budget_used);
            if alloc <= 0.0 {
                break;
            }

            requests.push(TransferRequest::new(
                next_id,
                *pos,
                TransferNeedPriority::Critical,
                TransferNeedReason::FormationGap,
                avg_ability.saturating_sub(10),
                avg_ability,
                alloc,
            ));
            next_id += 1;
            budget_used += alloc;
        }

        // Quality issues - IMPORTANT: player is significantly below squad level
        for (pos, _, _) in &quality_issues {
            let alloc = budget_per_need.min(available_budget - budget_used);
            if alloc <= 0.0 {
                break;
            }

            requests.push(TransferRequest::new(
                next_id,
                *pos,
                TransferNeedPriority::Important,
                TransferNeedReason::QualityUpgrade,
                avg_ability.saturating_sub(5),
                avg_ability + 5,
                alloc,
            ));
            next_id += 1;
            budget_used += alloc;
        }

        // Depth issues - OPTIONAL: need backup for a position group
        let mut depth_positions_handled: Vec<PlayerFieldPositionGroup> = Vec::new();
        for &&pos in &depth_issues {
            let group = pos.position_group();
            if depth_positions_handled.contains(&group) {
                continue;
            }
            depth_positions_handled.push(group);

            let alloc = (budget_per_need * 0.6).min(available_budget - budget_used);
            if alloc <= 0.0 {
                break;
            }

            requests.push(TransferRequest::new(
                next_id,
                pos,
                TransferNeedPriority::Optional,
                TransferNeedReason::DepthCover,
                avg_ability.saturating_sub(15),
                avg_ability.saturating_sub(5),
                alloc,
            ));
            next_id += 1;
            budget_used += alloc;
        }

        // ──────────────────────────────────────────────────────────
        // STEP 4: Succession planning for aging key players
        // ──────────────────────────────────────────────────────────

        // Only elite/continental clubs plan ahead
        if matches!(rep_level, ReputationLevel::Elite | ReputationLevel::Continental) {
            for player_info in &squad {
                if player_info.age >= 30
                    && player_info.current_ability >= avg_ability
                    && budget_used < available_budget
                {
                    let alloc = (budget_per_need * 0.4).min(available_budget - budget_used);
                    if alloc <= 0.0 {
                        break;
                    }

                    // Only if we don't already have a request for this position
                    if !requests.iter().any(|r| r.position == player_info.primary_position) {
                        requests.push(TransferRequest::new(
                            next_id,
                            player_info.primary_position,
                            TransferNeedPriority::Optional,
                            TransferNeedReason::SuccessionPlanning,
                            avg_ability.saturating_sub(10),
                            avg_ability,
                            alloc,
                        ));
                        next_id += 1;
                        budget_used += alloc;
                    }
                }
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 5: Identify loan-out candidates (reputation-driven)
        // ──────────────────────────────────────────────────────────

        Self::identify_loan_outs(
            &squad,
            &rep_level,
            avg_ability,
            date,
            players,
            &mut loan_outs,
        );

        let _ = next_id;

        SquadEvaluation {
            club_id: club.id,
            requests,
            loan_outs,
            total_budget: budget,
            max_concurrent,
        }
    }

    /// Identify loan-out candidates based on club reputation tier.
    /// Real-world patterns:
    /// - Elite clubs (Man City, Real Madrid): Loan out young players who need game time,
    ///   players blocked by star signings
    /// - Continental clubs: Loan out surplus players, young talent
    /// - National/Regional: Rarely loan out, might loan surplus
    fn identify_loan_outs(
        squad: &[SquadPlayerInfo],
        rep_level: &ReputationLevel,
        avg_ability: u8,
        _date: NaiveDate,
        players: &[Player],
        loan_outs: &mut Vec<LoanOutCandidate>,
    ) {
        for player_info in squad {
            let player = match players.iter().find(|p| p.id == player_info.player_id) {
                Some(p) => p,
                None => continue,
            };

            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Lst)
                || statuses.contains(&PlayerStatusType::Loa)
            {
                continue;
            }

            let age = player_info.age;
            let ability = player_info.current_ability;
            let potential = player_info.potential_ability;

            match rep_level {
                ReputationLevel::Elite | ReputationLevel::Continental => {
                    // Young players with high potential but not ready for first team
                    // This is the "Chelsea/Man City academy" pattern
                    if age <= 22 && potential > ability + 8 && ability < avg_ability {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                        });
                        continue;
                    }

                    // Good players blocked by better players in same position
                    // E.g., a 75-rated CB when the club has two 85+ rated CBs
                    if age >= 21
                        && age <= 28
                        && ability < avg_ability
                        && ability >= avg_ability.saturating_sub(15)
                    {
                        // Check if there are better players in same position
                        let same_pos_better = squad
                            .iter()
                            .filter(|p| {
                                p.player_id != player_info.player_id
                                    && p.primary_position == player_info.primary_position
                                    && p.current_ability > ability + 5
                            })
                            .count();

                        if same_pos_better >= 2 {
                            loan_outs.push(LoanOutCandidate {
                                player_id: player_info.player_id,
                                reason: LoanOutReason::BlockedByBetterPlayer,
                                status: LoanOutStatus::Identified,
                            });
                            continue;
                        }
                    }
                }
                ReputationLevel::National => {
                    // National clubs loan out young players who aren't quite ready
                    if age <= 20 && potential > ability + 10 && ability < avg_ability.saturating_sub(10) {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                        });
                        continue;
                    }
                }
                _ => {
                    // Regional/Local/Amateur clubs rarely loan out
                }
            }

            // All tiers: genuine surplus (too many in one position group)
            let group = player_info.primary_position.position_group();
            let group_count = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .count();

            let surplus_threshold = match group {
                PlayerFieldPositionGroup::Goalkeeper => 3,
                PlayerFieldPositionGroup::Defender => 8,
                PlayerFieldPositionGroup::Midfielder => 8,
                PlayerFieldPositionGroup::Forward => 5,
            };

            if group_count > surplus_threshold && ability < avg_ability {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::Surplus,
                    status: LoanOutStatus::Identified,
                });
            }
        }
    }

    /// Get formation positions, falling back to T442 for unmapped formations.
    fn get_formation_positions(formation: MatchTacticType) -> &'static [PlayerPositionType; 11] {
        let (_, positions) = TACTICS_POSITIONS
            .iter()
            .find(|(tactic, _)| *tactic == formation)
            .unwrap_or(&TACTICS_POSITIONS[0]);
        positions
    }

    // ============================================================
    // Step 3: Scout Assignment
    // ============================================================

    pub fn assign_scouts(country: &mut Country, _date: NaiveDate) {
        let mut actions: Vec<ScoutAssignmentAction> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let assigned_request_ids: Vec<u32> = plan
                .scouting_assignments
                .iter()
                .map(|a| a.transfer_request_id)
                .collect();

            let pending_requests: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status == TransferRequestStatus::Pending
                        && !assigned_request_ids.contains(&r.id)
                })
                .collect();

            if pending_requests.is_empty() {
                continue;
            }

            if club.teams.teams.is_empty() {
                continue;
            }
            let resolved = StaffResolver::resolve(&club.teams.teams[0].staffs);

            let mut sorted_requests = pending_requests;
            sorted_requests.sort_by(|a, b| {
                let priority_order = |p: &TransferNeedPriority| match p {
                    TransferNeedPriority::Critical => 0,
                    TransferNeedPriority::Important => 1,
                    TransferNeedPriority::Optional => 2,
                };
                priority_order(&a.priority).cmp(&priority_order(&b.priority))
            });

            let mut scout_idx = 0;
            let next_assign_id = plan.next_assignment_id;

            for (i, request) in sorted_requests.iter().enumerate() {
                let scout_id = if !resolved.scouts.is_empty() {
                    let s = resolved.scouts[scout_idx % resolved.scouts.len()];
                    scout_idx += 1;
                    Some(s.id)
                } else {
                    None
                };

                let assignment = ScoutingAssignment::new(
                    next_assign_id + i as u32,
                    request.id,
                    scout_id,
                    request.position.clone(),
                    request.min_ability,
                    request.preferred_age_min,
                    request.preferred_age_max,
                    request.budget_allocation,
                );

                actions.push(ScoutAssignmentAction {
                    club_id: club.id,
                    assignment,
                    request_id: request.id,
                });
            }
        }

        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let plan = &mut club.transfer_plan;

                if let Some(req) = plan
                    .transfer_requests
                    .iter_mut()
                    .find(|r| r.id == action.request_id)
                {
                    req.status = TransferRequestStatus::ScoutingActive;
                }

                plan.next_assignment_id = action.assignment.id + 1;
                plan.scouting_assignments.push(action.assignment);
            }
        }
    }

    // ============================================================
    // Step 4: Scouting Observations
    // ============================================================

    pub fn process_scouting(country: &mut Country, date: NaiveDate) {
        let price_level = country.settings.pricing.price_level;

        let mut all_players: Vec<PlayerSummary> = Vec::new();

        for club in &country.clubs {
            for team in &club.teams.teams {
                for player in &team.players.players {
                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                    );
                    let statuses = player.statuses.get();
                    all_players.push(PlayerSummary {
                        player_id: player.id,
                        club_id: club.id,
                        position: player.position(),
                        position_group: player.position().position_group(),
                        current_ability: player.player_attributes.current_ability,
                        potential_ability: player.player_attributes.potential_ability,
                        age: player.age(date),
                        estimated_value: value.amount,
                        is_listed: statuses.contains(&PlayerStatusType::Lst),
                        is_loan_listed: statuses.contains(&PlayerStatusType::Loa),
                    });
                }
            }
        }

        let mut observations: Vec<ScoutingObservationResult> = Vec::new();
        let mut reports: Vec<ScoutingReportResult> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            for assignment in &plan.scouting_assignments {
                if assignment.completed {
                    continue;
                }

                let (judging_ability, judging_potential) = if let Some(scout_id) = assignment.scout_staff_id {
                    Self::get_scout_skills(club, scout_id)
                } else {
                    (8, 8)
                };

                let observe_chance = 40 + (judging_ability as i32 / 2);
                if IntegerUtils::random(0, 100) > observe_chance {
                    continue;
                }

                // Find matching players from OTHER clubs
                // Scouts look at position GROUP match, not just exact position
                let target_group = assignment.target_position.position_group();
                let matching: Vec<&PlayerSummary> = all_players
                    .iter()
                    .filter(|p| {
                        p.club_id != club.id
                            && p.position_group == target_group
                            && p.age >= assignment.preferred_age_min
                            && p.age <= assignment.preferred_age_max
                            && p.current_ability >= assignment.min_ability
                            && p.estimated_value <= assignment.max_budget * 1.5
                    })
                    .collect();

                if matching.is_empty() {
                    continue;
                }

                let idx = (IntegerUtils::random(0, matching.len() as i32) as usize)
                    .min(matching.len() - 1);
                let target = matching[idx];

                let existing_obs = assignment.observations.iter()
                    .find(|o| o.player_id == target.player_id);
                let obs_count = existing_obs.map(|o| o.observation_count).unwrap_or(0);
                let sqrt_count = ((obs_count + 1) as f32).sqrt();

                let base_ability_error = (20i16 - judging_ability as i16).max(1) as f32;
                let base_potential_error = (20i16 - judging_potential as i16).max(1) as f32;
                let ability_error = (base_ability_error / sqrt_count) as i32;
                let potential_error = (base_potential_error / sqrt_count) as i32;

                let assessed_ability = (target.current_ability as i32
                    + IntegerUtils::random(-ability_error, ability_error))
                    .clamp(1, 100) as u8;
                let assessed_potential = (target.potential_ability as i32
                    + IntegerUtils::random(-potential_error, potential_error))
                    .clamp(1, 100) as u8;

                let is_new = !assignment.has_observation_for(target.player_id);

                observations.push(ScoutingObservationResult {
                    club_id: club.id,
                    assignment_id: assignment.id,
                    player_id: target.player_id,
                    assessed_ability,
                    assessed_potential,
                    is_new,
                });

                let final_obs_count = obs_count + 1;
                if final_obs_count >= 3 {
                    let confidence = 1.0 - (1.0 / (final_obs_count as f32 + 1.0));

                    let recommendation = if assessed_ability as i16 >= assignment.min_ability as i16 + 10
                        && assessed_potential > assessed_ability + 5
                    {
                        ScoutingRecommendation::StrongBuy
                    } else if assessed_ability >= assignment.min_ability
                        && assessed_potential >= assessed_ability
                    {
                        ScoutingRecommendation::Buy
                    } else if assessed_ability >= assignment.min_ability.saturating_sub(5) {
                        ScoutingRecommendation::Consider
                    } else {
                        ScoutingRecommendation::Pass
                    };

                    if recommendation != ScoutingRecommendation::Pass {
                        reports.push(ScoutingReportResult {
                            club_id: club.id,
                            report: DetailedScoutingReport {
                                player_id: target.player_id,
                                assignment_id: assignment.id,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_value: target.estimated_value,
                                recommendation,
                            },
                            assignment_id: assignment.id,
                        });
                    }
                }
            }
        }

        // Pass 2: Apply observations and reports
        for obs in observations {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == obs.club_id) {
                if let Some(assignment) = club
                    .transfer_plan
                    .scouting_assignments
                    .iter_mut()
                    .find(|a| a.id == obs.assignment_id)
                {
                    if obs.is_new {
                        assignment.observations.push(
                            crate::transfers::pipeline::PlayerObservation::new(
                                obs.player_id,
                                obs.assessed_ability,
                                obs.assessed_potential,
                                date,
                            ),
                        );
                    } else if let Some(existing) = assignment.find_observation_mut(obs.player_id) {
                        existing.add_observation(obs.assessed_ability, obs.assessed_potential, date);
                    }
                }
            }
        }

        for report in reports {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == report.club_id) {
                if !club
                    .transfer_plan
                    .scouting_reports
                    .iter()
                    .any(|r| r.player_id == report.report.player_id && r.assignment_id == report.assignment_id)
                {
                    club.transfer_plan.scouting_reports.push(report.report);

                    if let Some(assignment) = club
                        .transfer_plan
                        .scouting_assignments
                        .iter_mut()
                        .find(|a| a.id == report.assignment_id)
                    {
                        assignment.reports_produced += 1;
                        if assignment.reports_produced >= 3 {
                            assignment.completed = true;
                        }
                    }
                }
            }
        }
    }

    // ============================================================
    // Step 5: Shortlist Building
    // ============================================================

    pub fn build_shortlists(country: &mut Country, _date: NaiveDate) {
        let mut results: Vec<ShortlistResult> = Vec::new();

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            let existing_shortlist_request_ids: Vec<u32> = plan
                .shortlists
                .iter()
                .map(|s| s.transfer_request_id)
                .collect();

            let completed_assignments: Vec<&ScoutingAssignment> = plan
                .scouting_assignments
                .iter()
                .filter(|a| a.completed)
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

                let mut candidates: Vec<ShortlistCandidate> = reports
                    .iter()
                    .filter(|r| r.estimated_value <= budget_alloc * 1.2)
                    .map(|r| {
                        let ability_score = r.assessed_ability as f32 / 100.0;
                        let potential_score = r.assessed_potential as f32 / 100.0;
                        let value_fit = if budget_alloc > 0.0 {
                            1.0 - (r.estimated_value / budget_alloc).min(1.0) as f32
                        } else {
                            0.5
                        };
                        let confidence_score = r.confidence;

                        let score = ability_score * 0.3
                            + potential_score * 0.2
                            + value_fit * 0.25
                            + confidence_score * 0.1
                            + match r.recommendation {
                                ScoutingRecommendation::StrongBuy => 0.15,
                                ScoutingRecommendation::Buy => 0.10,
                                ScoutingRecommendation::Consider => 0.05,
                                ScoutingRecommendation::Pass => 0.0,
                            };

                        ShortlistCandidate {
                            player_id: r.player_id,
                            score,
                            estimated_fee: r.estimated_value,
                            status: ShortlistCandidateStatus::Available,
                        }
                    })
                    .collect();

                candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
                candidates.truncate(5);

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

            // No-scout fallback
            if club.teams.teams.is_empty() {
                continue;
            }
            let resolved = StaffResolver::resolve(&club.teams.teams[0].staffs);

            if !resolved.has_dedicated_scouts() {
                for request in &plan.transfer_requests {
                    if request.status != TransferRequestStatus::Pending
                        && request.status != TransferRequestStatus::ScoutingActive
                    {
                        continue;
                    }
                    if existing_shortlist_request_ids.contains(&request.id) {
                        continue;
                    }
                    let has_assignment = plan
                        .scouting_assignments
                        .iter()
                        .any(|a| a.transfer_request_id == request.id);
                    if has_assignment {
                        continue;
                    }

                    let market_candidates: Vec<ShortlistCandidate> = country
                        .transfer_market
                        .get_available_listings()
                        .iter()
                        .filter(|l| l.club_id != club.id)
                        .filter_map(|l| {
                            Self::find_player_summary_in_country(country, l.player_id).and_then(
                                |p| {
                                    if p.position_group == request.position.position_group()
                                        && p.current_ability >= request.min_ability
                                        && p.estimated_value <= request.budget_allocation * 1.5
                                    {
                                        Some(ShortlistCandidate {
                                            player_id: p.player_id,
                                            score: p.current_ability as f32 / 100.0,
                                            estimated_fee: p.estimated_value,
                                            status: ShortlistCandidateStatus::Available,
                                        })
                                    } else {
                                        None
                                    }
                                },
                            )
                        })
                        .take(5)
                        .collect();

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

    // ============================================================
    // Step 6: Negotiation Initiation - Smart buy/loan decisions
    // ============================================================

    pub fn initiate_negotiations(country: &mut Country, date: NaiveDate) {
        let mut actions: Vec<NegotiationAction> = Vec::new();
        let price_level = country.settings.pricing.price_level;

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            if !plan.initialized || !plan.can_start_negotiation() {
                continue;
            }

            let actual_active = country
                .transfer_market
                .active_negotiation_count_for_club(club.id);
            if actual_active >= plan.max_concurrent_negotiations {
                continue;
            }

            let budget = club
                .finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount)
                .unwrap_or(0.0);

            if club.teams.teams.is_empty() {
                continue;
            }

            let team = &club.teams.teams[0];
            let rep_level = team.reputation.level();

            let avg_ability: u8 = if !team.players.players.is_empty() {
                let total: u32 = team
                    .players
                    .players
                    .iter()
                    .map(|p| p.player_attributes.current_ability as u32)
                    .sum();
                (total / team.players.players.len() as u32) as u8
            } else {
                50
            };

            let buying_aggressiveness = match rep_level {
                ReputationLevel::Elite => 0.85,
                ReputationLevel::Continental => 0.75,
                ReputationLevel::National => 0.60,
                ReputationLevel::Regional => 0.45,
                _ => 0.30,
            };

            let slots_available =
                plan.max_concurrent_negotiations.saturating_sub(actual_active) as usize;
            let mut negotiations_this_club = 0usize;

            for shortlist in &plan.shortlists {
                if negotiations_this_club >= slots_available {
                    break;
                }

                if shortlist.has_pursuing_candidate() {
                    continue;
                }

                if shortlist.all_exhausted() {
                    continue;
                }

                let candidate = match shortlist.current_candidate() {
                    Some(c) if c.status == ShortlistCandidateStatus::Available => c,
                    _ => continue,
                };

                let player_id = candidate.player_id;

                if country
                    .transfer_market
                    .has_active_negotiation_for(player_id, club.id)
                {
                    continue;
                }

                let selling_club_id = country
                    .clubs
                    .iter()
                    .find(|c| {
                        c.teams
                            .teams
                            .iter()
                            .any(|t| t.players.players.iter().any(|p| p.id == player_id))
                    })
                    .map(|c| c.id);

                let selling_club_id = match selling_club_id {
                    Some(id) if id != club.id => id,
                    _ => continue,
                };

                // ──────────────────────────────────────────────────
                // SMART BUY/LOAN DECISION
                // The DoF decides the approach based on context:
                // - Club reputation tier
                // - Budget vs player value
                // - Transfer request reason
                // - Whether the player is loan-listed
                // - Player age and potential
                // ──────────────────────────────────────────────────

                let request = plan
                    .transfer_requests
                    .iter()
                    .find(|r| r.id == shortlist.transfer_request_id);

                let approach = Self::determine_transfer_approach(
                    &rep_level,
                    budget,
                    candidate.estimated_fee,
                    request,
                    country,
                    player_id,
                    date,
                );

                let is_loan = matches!(
                    approach,
                    TransferApproach::Loan | TransferApproach::LoanWithOption
                );

                if let Some(player) = Self::find_player_in_country(country, player_id) {
                    let strategy = ClubTransferStrategy {
                        club_id: club.id,
                        budget: Some(CurrencyValue {
                            amount: shortlist.allocated_budget.min(budget),
                            currency: Currency::Usd,
                        }),
                        selling_willingness: 0.5,
                        buying_aggressiveness,
                        target_positions: vec![player.position()],
                        reputation_level: avg_ability as u16,
                    };

                    let selling_club = country
                        .clubs
                        .iter()
                        .find(|c| c.id == selling_club_id)
                        .unwrap();

                    let asking_price = Self::calculate_asking_price(
                        player,
                        selling_club,
                        date,
                        price_level,
                    );

                    let actual_asking = if is_loan {
                        CurrencyValue {
                            amount: asking_price.amount * 0.1,
                            currency: asking_price.currency.clone(),
                        }
                    } else {
                        asking_price
                    };

                    let offer =
                        strategy.calculate_initial_offer(player, &actual_asking, date);

                    actions.push(NegotiationAction {
                        club_id: club.id,
                        player_id,
                        selling_club_id,
                        offer,
                        is_loan,
                        shortlist_request_id: shortlist.transfer_request_id,
                    });

                    negotiations_this_club += 1;
                }
            }

            // Process loan-out candidates
            for loan_candidate in &plan.loan_out_candidates {
                if loan_candidate.status != LoanOutStatus::Identified {
                    continue;
                }

                if let Some(player) = Self::find_player_in_club(club, loan_candidate.player_id) {
                    let asking_price = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                    );

                    let listing = TransferListing::new(
                        loan_candidate.player_id,
                        club.id,
                        team.id,
                        asking_price,
                        date,
                        TransferListingType::Loan,
                    );
                    let _ = listing;
                }
            }
        }

        // Pass 2: Start negotiations
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) = Self::get_player_negotiation_data(country, action.player_id, date);

            let has_listing = country
                .transfer_market
                .get_listing_by_player(action.player_id)
                .is_some();

            if !has_listing {
                let listing_type = if action.is_loan {
                    TransferListingType::Loan
                } else {
                    TransferListingType::Transfer
                };

                let selling_team_id = country
                    .clubs
                    .iter()
                    .find(|c| c.id == action.selling_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.id)
                    .unwrap_or(0);

                let asking = CurrencyValue {
                    amount: action.offer.base_fee.amount * 1.2,
                    currency: Currency::Usd,
                };

                let listing = TransferListing::new(
                    action.player_id,
                    action.selling_club_id,
                    selling_team_id,
                    asking,
                    date,
                    listing_type,
                );
                country.transfer_market.add_listing(listing);
            }

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.club_id,
                action.offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = action.is_loan;
                    negotiation.is_unsolicited = !has_listing;
                }

                if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                    let plan = &mut club.transfer_plan;

                    if let Some(shortlist) = plan
                        .shortlists
                        .iter_mut()
                        .find(|s| s.transfer_request_id == action.shortlist_request_id)
                    {
                        if let Some(candidate) = shortlist.current_candidate_mut() {
                            if candidate.player_id == action.player_id {
                                candidate.status = ShortlistCandidateStatus::CurrentlyPursuing;
                            }
                        }
                    }

                    if let Some(req) = plan
                        .transfer_requests
                        .iter_mut()
                        .find(|r| r.id == action.shortlist_request_id)
                    {
                        req.status = TransferRequestStatus::Negotiating;
                    }

                    plan.active_negotiation_count += 1;
                }

                debug!(
                    "Pipeline: Club {} started negotiation for player {} ({})",
                    action.club_id,
                    action.player_id,
                    if action.is_loan { "loan" } else { "transfer" }
                );
            }
        }

        Self::process_loan_out_listings(country, date);
    }

    /// Determine whether to buy or loan a player.
    /// This is the "DoF decision" - mirrors real-world logic:
    ///
    /// - Elite clubs: Buy starters, loan promising youngsters with options
    /// - Continental clubs: Buy key targets, loan when budget is tight
    /// - National clubs: Buy affordable targets, loan expensive ones
    /// - Regional/Local: Loan most players, only buy cheap or free agents
    /// - If player is loan-listed by their club: always loan
    /// - Development signings: always loan
    fn determine_transfer_approach(
        rep_level: &ReputationLevel,
        budget: f64,
        estimated_fee: f64,
        request: Option<&TransferRequest>,
        country: &Country,
        player_id: u32,
        _date: NaiveDate,
    ) -> TransferApproach {
        // If the player is already loan-listed, pursue a loan
        if let Some(player) = Self::find_player_in_country(country, player_id) {
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Loa) {
                return TransferApproach::Loan;
            }
        }

        // Development signings are always loans
        if let Some(req) = request {
            if req.reason == TransferNeedReason::DevelopmentSigning {
                return TransferApproach::Loan;
            }
        }

        // Can we even afford to buy?
        let affordability = if estimated_fee > 0.0 {
            budget / estimated_fee
        } else {
            10.0 // Free agent, always affordable
        };

        match rep_level {
            ReputationLevel::Elite => {
                // Elite clubs buy unless player is extremely expensive relative to budget
                if affordability >= 0.5 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::LoanWithOption
                }
            }
            ReputationLevel::Continental => {
                if affordability >= 0.7 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.3 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::National => {
                if affordability >= 1.0 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.5 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::Regional => {
                if affordability >= 2.0 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::Loan
                }
            }
            _ => {
                // Local/Amateur: almost always loan
                if affordability >= 3.0 && estimated_fee < 100_000.0 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::Loan
                }
            }
        }
    }

    /// List loan-out candidates on the transfer market.
    fn process_loan_out_listings(country: &mut Country, date: NaiveDate) {
        let price_level = country.settings.pricing.price_level;
        let mut listings_to_add: Vec<(u32, TransferListing)> = Vec::new();

        for club in &country.clubs {
            for candidate in &club.transfer_plan.loan_out_candidates {
                if candidate.status != LoanOutStatus::Identified {
                    continue;
                }

                if country
                    .transfer_market
                    .get_listing_by_player(candidate.player_id)
                    .is_some()
                {
                    continue;
                }

                if let Some(player) = Self::find_player_in_club(club, candidate.player_id) {
                    let team_id = club
                        .teams
                        .teams
                        .first()
                        .map(|t| t.id)
                        .unwrap_or(0);

                    let asking_price = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                    );

                    let listing = TransferListing::new(
                        candidate.player_id,
                        club.id,
                        team_id,
                        asking_price,
                        date,
                        TransferListingType::Loan,
                    );

                    listings_to_add.push((club.id, listing));
                }
            }
        }

        for (club_id, listing) in listings_to_add {
            let player_id = listing.player_id;
            country.transfer_market.add_listing(listing);

            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                if let Some(candidate) = club
                    .transfer_plan
                    .loan_out_candidates
                    .iter_mut()
                    .find(|c| c.player_id == player_id)
                {
                    candidate.status = LoanOutStatus::Listed;
                }

                for team in &mut club.teams.teams {
                    if let Some(player) = team
                        .players
                        .players
                        .iter_mut()
                        .find(|p| p.id == player_id)
                    {
                        if !player.statuses.get().contains(&PlayerStatusType::Loa) {
                            player.statuses.add(date, PlayerStatusType::Loa);
                        }
                    }
                }
            }
        }
    }

    // ============================================================
    // Negotiation Outcome Callback
    // ============================================================

    pub fn on_negotiation_resolved(
        country: &mut Country,
        buying_club_id: u32,
        player_id: u32,
        accepted: bool,
    ) {
        if let Some(club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            let plan = &mut club.transfer_plan;

            for shortlist in &mut plan.shortlists {
                if let Some(candidate) = shortlist
                    .candidates
                    .iter_mut()
                    .find(|c| c.player_id == player_id)
                {
                    if accepted {
                        candidate.status = ShortlistCandidateStatus::Signed;

                        if let Some(req) = plan
                            .transfer_requests
                            .iter_mut()
                            .find(|r| r.id == shortlist.transfer_request_id)
                        {
                            req.status = TransferRequestStatus::Fulfilled;
                        }
                    } else {
                        candidate.status = ShortlistCandidateStatus::NegotiationFailed;
                        shortlist.advance_to_next();

                        if shortlist.all_exhausted() {
                            if let Some(req) = plan
                                .transfer_requests
                                .iter_mut()
                                .find(|r| r.id == shortlist.transfer_request_id)
                            {
                                if req.priority == TransferNeedPriority::Critical {
                                    req.status = TransferRequestStatus::Pending;
                                } else {
                                    req.status = TransferRequestStatus::Abandoned;
                                }
                            }
                        } else {
                            if let Some(req) = plan
                                .transfer_requests
                                .iter_mut()
                                .find(|r| r.id == shortlist.transfer_request_id)
                            {
                                req.status = TransferRequestStatus::Shortlisted;
                            }
                        }
                    }

                    break;
                }
            }

            plan.active_negotiation_count = plan.active_negotiation_count.saturating_sub(1);
        }
    }

    // ============================================================
    // Helper methods
    // ============================================================

    fn find_player_in_country<'a>(country: &'a Country, player_id: u32) -> Option<&'a Player> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                    return Some(player);
                }
            }
        }
        None
    }

    fn find_player_in_club<'a>(club: &'a Club, player_id: u32) -> Option<&'a Player> {
        for team in &club.teams.teams {
            if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                return Some(player);
            }
        }
        None
    }

    fn find_player_summary_in_country(
        country: &Country,
        player_id: u32,
    ) -> Option<PlayerSummary> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                    let now = chrono::Local::now().naive_local().date();
                    return Some(PlayerSummary {
                        player_id: player.id,
                        club_id: club.id,
                        position: player.position(),
                        position_group: player.position().position_group(),
                        current_ability: player.player_attributes.current_ability,
                        potential_ability: player.player_attributes.potential_ability,
                        age: player.age(now),
                        estimated_value: player.player_attributes.current_ability as f64 * 10000.0,
                        is_listed: player.statuses.get().contains(&PlayerStatusType::Lst),
                        is_loan_listed: player.statuses.get().contains(&PlayerStatusType::Loa),
                    });
                }
            }
        }
        None
    }

    fn get_scout_skills(club: &Club, scout_id: u32) -> (u8, u8) {
        for team in &club.teams.teams {
            if let Some(staff) = team.staffs.staffs.iter().find(|s| s.id == scout_id) {
                return (
                    staff.staff_attributes.knowledge.judging_player_ability,
                    staff.staff_attributes.knowledge.judging_player_potential,
                );
            }
        }
        (10, 10)
    }

    fn calculate_asking_price(
        player: &Player,
        club: &Club,
        date: NaiveDate,
        price_level: f32,
    ) -> CurrencyValue {
        let base_value =
            PlayerValuationCalculator::calculate_value_with_price_level(player, date, price_level);

        let multiplier = if club.finance.balance.balance < 0 {
            0.9
        } else {
            1.1
        };

        CurrencyValue {
            amount: base_value.amount * multiplier,
            currency: base_value.currency,
        }
    }

    fn get_club_reputation(country: &Country, club_id: u32) -> f32 {
        country
            .clubs
            .iter()
            .find(|c| c.id == club_id)
            .and_then(|c| c.teams.teams.first())
            .map(|t| t.reputation.attractiveness_factor())
            .unwrap_or(0.3)
    }

    fn get_player_negotiation_data(
        country: &Country,
        player_id: u32,
        date: NaiveDate,
    ) -> (u8, f32) {
        Self::find_player_in_country(country, player_id)
            .map(|p| (p.age(date), p.attributes.ambition))
            .unwrap_or((25, 0.5))
    }
}
