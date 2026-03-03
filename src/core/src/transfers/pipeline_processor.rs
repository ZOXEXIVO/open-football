use chrono::{Datelike, NaiveDate};
use log::debug;
use std::collections::HashMap;

use crate::shared::{Currency, CurrencyValue};
use crate::transfers::market::{TransferListing, TransferListingType};
use crate::transfers::pipeline::{
    ClubTransferPlan, DetailedScoutingReport, LoanOutCandidate, LoanOutReason, LoanOutStatus,
    RecommendationSource, RecommendationType, ScoutMatchAssignment, ScoutingAssignment,
    ScoutingRecommendation, ShortlistCandidate, ShortlistCandidateStatus, StaffRecommendation,
    TransferApproach, TransferNeedPriority, TransferNeedReason, TransferRequest,
    TransferRequestStatus, TransferShortlist,
};
use crate::transfers::staff_resolver::StaffResolver;
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::IntegerUtils;
use crate::{
    Club, ClubTransferStrategy, Country, MatchTacticType, Person, Player,
    PlayerFieldPositionGroup, PlayerPositionType, PlayerStatusType, ReputationLevel,
    StaffEventType, TacticsSelector, TeamType, TACTICS_POSITIONS,
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

struct MatchScoutAssignmentAction {
    club_id: u32,
    assignment: ScoutMatchAssignment,
}

struct MatchScoutingObservationResult {
    club_id: u32,
    assignment_id: u32,
    player_id: u32,
    assessed_ability: u8,
    assessed_potential: u8,
    match_rating: f32,
    is_new: bool,
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
    appearances: u16,
    is_injured: bool,
    recovery_days: u16,
    #[allow(dead_code)]
    injury_days: u16,
}

impl PipelineProcessor {
    // ============================================================
    // Step 2: Squad Evaluation - Coach-driven, formation-based
    // ============================================================

    pub fn evaluate_squads(country: &mut Country, date: NaiveDate) {
        let is_window_start = Self::is_window_start(date);
        let should_evaluate = is_window_start || Self::should_evaluate(date);

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

        if !evaluations.is_empty() {
            debug!("Transfer pipeline: evaluated {} clubs", evaluations.len());
        }

        // Pass 2: Apply evaluations (mutable writes)
        for eval in evaluations {
            if !eval.requests.is_empty() {
                debug!(
                    "Transfer pipeline: Club {} has {} requests, {} loan-outs, budget={:.0}",
                    eval.club_id, eval.requests.len(), eval.loan_outs.len(), eval.total_budget
                );
            }

            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == eval.club_id) {
                let plan = &mut club.transfer_plan;

                // Only fully reset plans at window start or first initialization
                if !plan.initialized || is_window_start {
                    plan.reset_for_window();
                } else {
                    // On re-evaluation: remove completed assignments and exhausted shortlists
                    // so new requests can be scouted fresh
                    plan.scouting_assignments.retain(|a| !a.completed);
                    plan.shortlists.retain(|s| !s.all_exhausted());
                }

                plan.total_budget = eval.total_budget;
                plan.max_concurrent_negotiations = eval.max_concurrent;

                // Track the highest request ID so re-evaluations don't create duplicate IDs
                if let Some(max_id) = eval.requests.iter().map(|r| r.id).max() {
                    plan.next_request_id = max_id + 1;
                }

                plan.transfer_requests.extend(eval.requests);
                plan.loan_out_candidates.extend(eval.loan_outs);
                plan.last_evaluation_date = Some(date);
                plan.initialized = true;
            }
        }
    }

    /// Full reset only at transfer window opening dates
    fn is_window_start(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        (month == 5 && day == 31) || (month == 6 && day == 1) || (month == 1 && day == 1)
    }

    /// Re-evaluate during transfer windows.
    /// Daily during the first week of each window for fast pipeline startup,
    /// then weekly (Monday) for the rest of the window.
    fn should_evaluate(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();

        // First week of summer window (June 1-7) or winter window (Jan 1-7): daily
        if (month == 6 && day <= 7) || (month == 1 && day <= 7) {
            return true;
        }

        // Rest of window: weekly on Monday
        ((month >= 6 && month <= 8) || month == 1)
            && date.weekday() == chrono::Weekday::Mon
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
            ReputationLevel::Elite => 6,
            ReputationLevel::Continental => 5,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            _ => 2,
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
                    appearances: p.statistics.played + p.statistics.played_subs,
                    is_injured: p.player_attributes.is_injured,
                    recovery_days: p.player_attributes.recovery_days_remaining,
                    injury_days: p.player_attributes.injury_days_remaining,
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

        // ──────────────────────────────────────────────────────────
        // STEP 6: Small clubs proactively seek loan reinforcements
        // Real football: Regional/Local/Amateur clubs survive on loans.
        // Their staff actively look for loan deals to fill squads,
        // get experienced heads, cover injuries, and pad numbers.
        // ──────────────────────────────────────────────────────────

        if matches!(
            rep_level,
            ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
        ) {
            let squad_size = squad.len();
            let is_january = Self::is_january_window(date);

            // Squad too small — need bodies (< 20 players)
            if squad_size < 20 {
                let positions_needed = 20 - squad_size;
                let padding_count = positions_needed.min(3); // max 3 padding requests
                for i in 0..padding_count {
                    let alloc = 5_000.0; // minimal budget — expecting loans
                    // Cycle through positions that are thinnest
                    let weakest_group = [
                        PlayerFieldPositionGroup::Defender,
                        PlayerFieldPositionGroup::Midfielder,
                        PlayerFieldPositionGroup::Forward,
                    ];
                    let group = weakest_group[i % 3];
                    let pos = match group {
                        PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
                        PlayerFieldPositionGroup::Midfielder => PlayerPositionType::MidfielderCenter,
                        PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
                        _ => PlayerPositionType::MidfielderCenter,
                    };

                    if !requests.iter().any(|r| {
                        r.position.position_group() == group
                            && matches!(
                                r.reason,
                                TransferNeedReason::SquadPadding | TransferNeedReason::LoanToFillSquad
                            )
                    }) {
                        requests.push(TransferRequest::new(
                            next_id,
                            pos,
                            TransferNeedPriority::Important,
                            TransferNeedReason::SquadPadding,
                            avg_ability.saturating_sub(15),
                            avg_ability.saturating_sub(5),
                            alloc,
                        ));
                        next_id += 1;
                    }
                }
            }

            // Need experienced head (no players aged 28+ with decent ability)
            let experienced_count = squad
                .iter()
                .filter(|p| p.age >= 28 && p.current_ability >= avg_ability.saturating_sub(5))
                .count();
            if experienced_count == 0 && squad_size >= 11 {
                let weakest_pos = position_coverage
                    .iter()
                    .filter(|(_, pid, _)| pid.is_some())
                    .min_by_key(|(_, _, quality)| *quality)
                    .map(|(pos, _, _)| *pos)
                    .unwrap_or(PlayerPositionType::MidfielderCenter);

                if !requests
                    .iter()
                    .any(|r| r.reason == TransferNeedReason::ExperiencedHead)
                {
                    requests.push(TransferRequest::new(
                        next_id,
                        weakest_pos,
                        TransferNeedPriority::Optional,
                        TransferNeedReason::ExperiencedHead,
                        avg_ability.saturating_sub(8),
                        avg_ability,
                        10_000.0,
                    ));
                    next_id += 1;
                }
            }

            // Injury cover — any position group with injured starters and no backup
            for &formation_pos in formation_positions {
                let group = formation_pos.position_group();
                let injured_in_group = squad
                    .iter()
                    .filter(|p| {
                        p.primary_position.position_group() == group
                            && p.is_injured
                            && p.injury_days > 30
                    })
                    .count();
                let healthy_in_group = squad
                    .iter()
                    .filter(|p| {
                        p.primary_position.position_group() == group && !p.is_injured
                    })
                    .count();
                let formation_needs = formation_positions
                    .iter()
                    .filter(|p| p.position_group() == group)
                    .count();

                if injured_in_group > 0 && healthy_in_group < formation_needs {
                    if !requests.iter().any(|r| {
                        r.position.position_group() == group
                            && r.reason == TransferNeedReason::InjuryCoverLoan
                    }) {
                        requests.push(TransferRequest::new(
                            next_id,
                            formation_pos,
                            TransferNeedPriority::Important,
                            TransferNeedReason::InjuryCoverLoan,
                            avg_ability.saturating_sub(12),
                            avg_ability.saturating_sub(3),
                            8_000.0,
                        ));
                        next_id += 1;
                    }
                }
            }

            // Cheap reinforcements — for each position group where quality is weak
            // Small clubs always looking for bargains
            for &(pos, _, quality) in &position_coverage {
                if quality > 0 && (quality as i16) < avg_ability as i16 - 8 {
                    let group = pos.position_group();
                    if !requests.iter().any(|r| {
                        r.position.position_group() == group
                            && matches!(
                                r.reason,
                                TransferNeedReason::CheapReinforcement
                                    | TransferNeedReason::QualityUpgrade
                                    | TransferNeedReason::LoanToFillSquad
                            )
                    }) {
                        requests.push(TransferRequest::new(
                            next_id,
                            pos,
                            TransferNeedPriority::Optional,
                            TransferNeedReason::CheapReinforcement,
                            avg_ability.saturating_sub(10),
                            avg_ability,
                            6_000.0,
                        ));
                        next_id += 1;
                    }
                }
            }

            // January-specific: actively seek loan deals for every unfilled formation slot
            if is_january {
                for &(pos, player_id, _) in &position_coverage {
                    if player_id.is_none() {
                        if !requests.iter().any(|r| {
                            r.position == pos
                                && matches!(
                                    r.reason,
                                    TransferNeedReason::LoanToFillSquad
                                        | TransferNeedReason::FormationGap
                                )
                        }) {
                            requests.push(TransferRequest::new(
                                next_id,
                                pos,
                                TransferNeedPriority::Critical,
                                TransferNeedReason::LoanToFillSquad,
                                avg_ability.saturating_sub(15),
                                avg_ability.saturating_sub(5),
                                5_000.0,
                            ));
                            next_id += 1;
                        }
                    }
                }
            }
        }

        // National clubs also seek some loan-based reinforcement (less aggressive)
        if rep_level == ReputationLevel::National {
            let squad_size = squad.len();
            let is_january = Self::is_january_window(date);

            // Thin squad
            if squad_size < 22 && is_january {
                let thinnest_group = [
                    PlayerFieldPositionGroup::Defender,
                    PlayerFieldPositionGroup::Midfielder,
                    PlayerFieldPositionGroup::Forward,
                ]
                .iter()
                .min_by_key(|&&group| {
                    squad
                        .iter()
                        .filter(|p| p.primary_position.position_group() == group)
                        .count()
                })
                .copied()
                .unwrap_or(PlayerFieldPositionGroup::Midfielder);

                let pos = match thinnest_group {
                    PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
                    PlayerFieldPositionGroup::Midfielder => PlayerPositionType::MidfielderCenter,
                    PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
                    _ => PlayerPositionType::MidfielderCenter,
                };

                if !requests
                    .iter()
                    .any(|r| r.reason == TransferNeedReason::LoanToFillSquad)
                {
                    requests.push(TransferRequest::new(
                        next_id,
                        pos,
                        TransferNeedPriority::Optional,
                        TransferNeedReason::LoanToFillSquad,
                        avg_ability.saturating_sub(10),
                        avg_ability.saturating_sub(3),
                        15_000.0,
                    ));
                    next_id += 1;
                }
            }

            // Injury cover for national clubs too
            for &formation_pos in formation_positions {
                let group = formation_pos.position_group();
                let injured_starters = squad
                    .iter()
                    .filter(|p| {
                        p.primary_position.position_group() == group
                            && p.is_injured
                            && p.injury_days > 45
                            && p.current_ability >= avg_ability.saturating_sub(5)
                    })
                    .count();

                if injured_starters > 0 && is_january {
                    if !requests.iter().any(|r| {
                        r.position.position_group() == group
                            && r.reason == TransferNeedReason::InjuryCoverLoan
                    }) {
                        requests.push(TransferRequest::new(
                            next_id,
                            formation_pos,
                            TransferNeedPriority::Important,
                            TransferNeedReason::InjuryCoverLoan,
                            avg_ability.saturating_sub(8),
                            avg_ability,
                            20_000.0,
                        ));
                        next_id += 1;
                    }
                }
            }
        }

        let _ = next_id;

        SquadEvaluation {
            club_id: club.id,
            requests,
            loan_outs,
            total_budget: budget,
            max_concurrent,
        }
    }

    fn is_january_window(date: NaiveDate) -> bool {
        date.month() == 1
    }

    /// Identify loan-out candidates based on club reputation tier.
    /// Real-world patterns:
    /// - Elite clubs (Man City, Real Madrid): Loan out young players who need game time,
    ///   players blocked by star signings, players lacking minutes, post-injury fitness
    /// - Continental clubs: Similar to elite but slightly less aggressive
    /// - National/Regional: Loan out young players, rarely others
    fn identify_loan_outs(
        squad: &[SquadPlayerInfo],
        rep_level: &ReputationLevel,
        avg_ability: u8,
        date: NaiveDate,
        players: &[Player],
        loan_outs: &mut Vec<LoanOutCandidate>,
    ) {
        let is_january = Self::is_january_window(date);

        // Calculate expected appearances based on season progress
        // Season runs Aug(1) through May(10), so Jan = month 6
        let month = date.month();
        let season_month = if month >= 8 { month - 7 } else { month + 5 };
        let expected_appearances = (season_month * 4) as u16;
        let barely_plays = (expected_appearances as f32 * 0.15) as u16;
        let starter_threshold = (expected_appearances as f32 * 0.50) as u16;

        for player_info in squad {
            let player = match players.iter().find(|p| p.id == player_info.player_id) {
                Some(p) => p,
                None => continue,
            };

            // Skip players already on loan from another club
            let is_on_loan = player.contract.as_ref()
                .map(|c| c.contract_type == crate::ContractType::Loan)
                .unwrap_or(false);
            if is_on_loan {
                continue;
            }

            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Lst)
                || statuses.contains(&PlayerStatusType::Loa)
            {
                continue;
            }

            // Skip key players
            let is_key_player = player
                .contract
                .as_ref()
                .map(|c| matches!(c.squad_status, crate::PlayerSquadStatus::KeyPlayer))
                .unwrap_or(false);

            let age = player_info.age;
            let ability = player_info.current_ability;
            let potential = player_info.potential_ability;
            let apps = player_info.appearances;

            // ── Pass A: Post-injury fitness loan (Elite/Continental/National) ──
            if matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental | ReputationLevel::National
            ) && player_info.recovery_days > 0
                && player_info.recovery_days <= 30
                && !player_info.is_injured
                && age <= 30
                && !is_key_player
            {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::PostInjuryFitness,
                    status: LoanOutStatus::Identified,
                });
                continue;
            }

            // ── Pass B: Lack of playing time (January window only) ──
            if is_january && expected_appearances > 0 {
                match rep_level {
                    ReputationLevel::Elite | ReputationLevel::Continental => {
                        // Players barely playing who are good enough to deserve minutes
                        if apps <= barely_plays
                            && ability >= avg_ability.saturating_sub(15)
                            && age <= 28
                            && !is_key_player
                        {
                            loan_outs.push(LoanOutCandidate {
                                player_id: player_info.player_id,
                                reason: LoanOutReason::LackOfPlayingTime,
                                status: LoanOutStatus::Identified,
                            });
                            continue;
                        }
                        // Players aged 23-25 not getting enough starts, below avg but decent
                        if age >= 23
                            && age <= 25
                            && apps < starter_threshold
                            && ability < avg_ability
                            && ability >= avg_ability.saturating_sub(20)
                        {
                            loan_outs.push(LoanOutCandidate {
                                player_id: player_info.player_id,
                                reason: LoanOutReason::LackOfPlayingTime,
                                status: LoanOutStatus::Identified,
                            });
                            continue;
                        }
                    }
                    ReputationLevel::National => {
                        if apps <= barely_plays
                            && age <= 25
                            && ability >= avg_ability.saturating_sub(12)
                            && !is_key_player
                        {
                            loan_outs.push(LoanOutCandidate {
                                player_id: player_info.player_id,
                                reason: LoanOutReason::LackOfPlayingTime,
                                status: LoanOutStatus::Identified,
                            });
                            continue;
                        }
                    }
                    _ => {}
                }
            }

            // ── Existing logic (relaxed thresholds) ──
            match rep_level {
                ReputationLevel::Elite | ReputationLevel::Continental => {
                    // Young high-potential: age limit 22→23, potential gap 8→5
                    if age <= 23 && potential > ability + 5 && ability < avg_ability {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                        });
                        continue;
                    }

                    // Blocked by better: threshold from 2 to 1 if player barely plays
                    if age >= 21
                        && age <= 28
                        && ability < avg_ability
                        && ability >= avg_ability.saturating_sub(15)
                    {
                        let same_pos_better = squad
                            .iter()
                            .filter(|p| {
                                p.player_id != player_info.player_id
                                    && p.primary_position == player_info.primary_position
                                    && p.current_ability > ability + 5
                            })
                            .count();

                        let block_threshold =
                            if is_january && apps <= barely_plays { 1 } else { 2 };

                        if same_pos_better >= block_threshold {
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
                    if age <= 22 && potential > ability + 8 && ability < avg_ability {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                        });
                        continue;
                    }
                }
                ReputationLevel::Regional => {
                    if age <= 21
                        && potential > ability + 10
                        && ability < avg_ability.saturating_sub(5)
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                        });
                        continue;
                    }
                }
                _ => {}
            }

            // All tiers: genuine surplus (too many in one position group)
            // January surplus thresholds lowered: DEF/MID 6→5, FWD 4→3
            let group = player_info.primary_position.position_group();
            let group_count = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .count();

            let surplus_threshold = if is_january {
                match group {
                    PlayerFieldPositionGroup::Goalkeeper => 3,
                    PlayerFieldPositionGroup::Defender => 5,
                    PlayerFieldPositionGroup::Midfielder => 5,
                    PlayerFieldPositionGroup::Forward => 3,
                }
            } else {
                match group {
                    PlayerFieldPositionGroup::Goalkeeper => 3,
                    PlayerFieldPositionGroup::Defender => 6,
                    PlayerFieldPositionGroup::Midfielder => 6,
                    PlayerFieldPositionGroup::Forward => 4,
                }
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
    // Step 3.5: Assign Scouts to Youth/Reserve Matches
    // ============================================================

    pub fn assign_scouts_to_matches(country: &mut Country, current_date: NaiveDate) {
        let mut actions: Vec<MatchScoutAssignmentAction> = Vec::new();

        // Pass 1: Immutable reads - determine which scouts to assign where
        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Get active scouting assignments to know what positions/ages we're looking for
            let active_assignments: Vec<&ScoutingAssignment> = plan
                .scouting_assignments
                .iter()
                .filter(|a| !a.completed)
                .collect();

            if active_assignments.is_empty() {
                continue;
            }

            if club.teams.teams.is_empty() {
                continue;
            }

            let resolved = StaffResolver::resolve(&club.teams.teams[0].staffs);
            if resolved.scouts.is_empty() {
                continue;
            }

            // Check existing match assignments - don't re-assign scouts already watching a team
            let already_assigned_scout_ids: Vec<u32> = plan
                .scout_match_assignments
                .iter()
                .filter(|a| {
                    a.last_attended
                        .map(|d| (current_date - d).num_days() < 7)
                        .unwrap_or(false)
                })
                .map(|a| a.scout_staff_id)
                .collect();

            let available_scouts: Vec<u32> = resolved
                .scouts
                .iter()
                .map(|s| s.id)
                .filter(|id| !already_assigned_scout_ids.contains(id))
                .collect();

            if available_scouts.is_empty() {
                continue;
            }

            let max_assignments = available_scouts.len().min(3);

            // Score each youth/reserve team from other clubs by how many matching players it has
            let mut team_scores: Vec<(u32, u32, u32, usize)> = Vec::new(); // (team_id, club_id, team_idx_for_ref, score)

            for other_club in &country.clubs {
                if other_club.id == club.id {
                    continue;
                }

                for team in &other_club.teams.teams {
                    // Only consider non-Main teams
                    if matches!(team.team_type, TeamType::Main) {
                        continue;
                    }

                    // Skip teams already being watched (within 7 days)
                    let already_watching = plan.scout_match_assignments.iter().any(|a| {
                        a.target_team_id == team.id
                            && a.last_attended
                                .map(|d| (current_date - d).num_days() < 7)
                                .unwrap_or(false)
                    });
                    if already_watching {
                        continue;
                    }

                    // Score: count how many players match any active scouting assignment criteria
                    let mut score = 0usize;
                    for player in &team.players.players {
                        let player_pos_group = player.position().position_group();
                        let player_age = player.age(current_date);

                        for assignment in &active_assignments {
                            let target_group = assignment.target_position.position_group();
                            if player_pos_group == target_group
                                && player_age >= assignment.preferred_age_min
                                && player_age <= assignment.preferred_age_max
                            {
                                score += 1;
                                break;
                            }
                        }
                    }

                    if score > 0 {
                        team_scores.push((team.id, other_club.id, 0, score));
                    }
                }
            }

            // Sort by score descending
            team_scores.sort_by(|a, b| b.3.cmp(&a.3));

            // Assign scouts to the best-scoring teams
            let assignments_to_make = team_scores.len().min(max_assignments);
            for i in 0..assignments_to_make {
                let (target_team_id, target_club_id, _, _) = team_scores[i];
                let scout_id = available_scouts[i];

                // Link to relevant scouting assignment IDs
                let linked_ids: Vec<u32> = active_assignments
                    .iter()
                    .map(|a| a.id)
                    .collect();

                actions.push(MatchScoutAssignmentAction {
                    club_id: club.id,
                    assignment: ScoutMatchAssignment {
                        scout_staff_id: scout_id,
                        target_team_id,
                        target_club_id,
                        linked_assignment_ids: linked_ids,
                        last_attended: None,
                    },
                });
            }
        }

        // Pass 2: Apply assignments
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                // Check if we already have an assignment for this team, update it
                if let Some(existing) = club
                    .transfer_plan
                    .scout_match_assignments
                    .iter_mut()
                    .find(|a| a.target_team_id == action.assignment.target_team_id)
                {
                    existing.scout_staff_id = action.assignment.scout_staff_id;
                    existing.linked_assignment_ids = action.assignment.linked_assignment_ids;
                } else {
                    club.transfer_plan
                        .scout_match_assignments
                        .push(action.assignment);
                }
            }
        }

        debug!("assign_scouts_to_matches: completed scout-to-match assignments");
    }

    // ============================================================
    // Step 3.75: Process Match-Day Scouting Observations
    // ============================================================

    pub fn process_match_scouting(country: &mut Country, current_date: NaiveDate) {
        let mut observations: Vec<MatchScoutingObservationResult> = Vec::new();
        let mut reports: Vec<ScoutingReportResult> = Vec::new();
        let mut attended_updates: Vec<(u32, u32, NaiveDate)> = Vec::new(); // (club_id, team_id, date)
        let mut staff_events: Vec<(u32, u32, StaffEventType)> = Vec::new(); // (club_id, staff_id, event)

        // Pass 1: Immutable reads
        for club in &country.clubs {
            let plan = &club.transfer_plan;

            for match_assignment in &plan.scout_match_assignments {
                // Find the target team and check if it played today
                let target_team = country.clubs.iter()
                    .find(|c| c.id == match_assignment.target_club_id)
                    .and_then(|c| c.teams.teams.iter().find(|t| t.id == match_assignment.target_team_id));

                let target_team = match target_team {
                    Some(t) => t,
                    None => continue,
                };

                // Check if this team played today
                let played_today = target_team
                    .match_history
                    .items()
                    .last()
                    .map(|m| m.date.date() == current_date)
                    .unwrap_or(false);

                if !played_today {
                    continue;
                }

                // Get scout skills
                let (judging_ability, judging_potential) =
                    Self::get_scout_skills(club, match_assignment.scout_staff_id);

                // Mark attendance
                attended_updates.push((club.id, match_assignment.target_team_id, current_date));
                staff_events.push((club.id, match_assignment.scout_staff_id, StaffEventType::MatchObserved));

                // Observe all players on the target team
                for player in &target_team.players.players {
                    let player_pos_group = player.position().position_group();
                    let player_age = player.age(current_date);
                    let match_rating = player.statistics.average_rating;

                    // Check if this player matches any linked scouting assignment
                    let matching_assignment = plan.scouting_assignments.iter().find(|a| {
                        !a.completed
                            && match_assignment.linked_assignment_ids.contains(&a.id)
                            && a.target_position.position_group() == player_pos_group
                            && player_age >= a.preferred_age_min
                            && player_age <= a.preferred_age_max
                    });

                    let assignment = match matching_assignment {
                        Some(a) => a,
                        None => continue,
                    };

                    // Calculate assessed ability/potential with 40% less error than pool scanning
                    let existing_obs = assignment.observations.iter()
                        .find(|o| o.player_id == player.id);
                    let obs_count = existing_obs.map(|o| o.observation_count).unwrap_or(0);
                    let sqrt_count = ((obs_count + 1) as f32).sqrt();

                    let base_ability_error = (20i16 - judging_ability as i16).max(1) as f32;
                    let base_potential_error = (20i16 - judging_potential as i16).max(1) as f32;
                    // 40% less error for match-context observations
                    let ability_error = ((base_ability_error * 0.6) / sqrt_count) as i32;
                    let potential_error = ((base_potential_error * 0.6) / sqrt_count) as i32;

                    let assessed_ability = (player.player_attributes.current_ability as i32
                        + IntegerUtils::random(-ability_error, ability_error))
                        .clamp(1, 100) as u8;
                    let assessed_potential = (player.player_attributes.potential_ability as i32
                        + IntegerUtils::random(-potential_error, potential_error))
                        .clamp(1, 100) as u8;

                    let is_new = !assignment.has_observation_for(player.id);

                    observations.push(MatchScoutingObservationResult {
                        club_id: club.id,
                        assignment_id: assignment.id,
                        player_id: player.id,
                        assessed_ability,
                        assessed_potential,
                        match_rating,
                        is_new,
                    });

                    // Generate report at 2+ observations
                    let final_obs_count = obs_count + 1;
                    if final_obs_count >= 2 {
                        let confidence = (1.0 - (0.5 / (final_obs_count as f32 + 1.0))).min(1.0);

                        // Match rating influences recommendation tier
                        let rating_boost = match_rating > 7.0;
                        let rating_penalty = match_rating < 5.5;

                        let recommendation = if rating_penalty {
                            // Low match rating downgrades
                            if assessed_ability >= assignment.min_ability {
                                ScoutingRecommendation::Consider
                            } else {
                                ScoutingRecommendation::Pass
                            }
                        } else if rating_boost
                            && assessed_ability as i16 >= assignment.min_ability as i16 + 5
                            && assessed_potential > assessed_ability
                        {
                            ScoutingRecommendation::StrongBuy
                        } else if assessed_ability as i16 >= assignment.min_ability as i16 + 10
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
                            let estimated_value = PlayerValuationCalculator::calculate_value_with_price_level(
                                player,
                                current_date,
                                country.settings.pricing.price_level,
                            );

                            reports.push(ScoutingReportResult {
                                club_id: club.id,
                                report: DetailedScoutingReport {
                                    player_id: player.id,
                                    assignment_id: assignment.id,
                                    assessed_ability,
                                    assessed_potential,
                                    confidence,
                                    estimated_value: estimated_value.amount,
                                    recommendation,
                                },
                                assignment_id: assignment.id,
                            });
                        }
                    }
                }
            }
        }

        // Pass 2: Apply observations, reports, and attendance updates
        for obs in observations {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == obs.club_id) {
                if let Some(assignment) = club
                    .transfer_plan
                    .scouting_assignments
                    .iter_mut()
                    .find(|a| a.id == obs.assignment_id)
                {
                    if obs.is_new {
                        let mut new_obs = crate::transfers::pipeline::PlayerObservation::new(
                            obs.player_id,
                            obs.assessed_ability,
                            obs.assessed_potential,
                            current_date,
                        );
                        // Start match observations at higher confidence
                        new_obs.confidence = 0.5;
                        assignment.observations.push(new_obs);
                    } else if let Some(existing) = assignment.find_observation_mut(obs.player_id) {
                        existing.add_match_observation(
                            obs.assessed_ability,
                            obs.assessed_potential,
                            obs.match_rating,
                            current_date,
                        );
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
                        if assignment.reports_produced >= 2 {
                            assignment.completed = true;
                        }
                    }
                }
            }
        }

        // Update last_attended dates
        for (club_id, team_id, date) in attended_updates {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                if let Some(match_assign) = club
                    .transfer_plan
                    .scout_match_assignments
                    .iter_mut()
                    .find(|a| a.target_team_id == team_id)
                {
                    match_assign.last_attended = Some(date);
                }
            }
        }

        // Push staff events for scouts
        for (club_id, staff_id, event_type) in staff_events {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                for team in &mut club.teams.teams {
                    if let Some(staff) = team.staffs.staffs.iter_mut().find(|s| s.id == staff_id) {
                        staff.add_event(event_type);
                        break;
                    }
                }
            }
        }

        debug!("process_match_scouting: completed match-day observations");
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
                    // Skip loan players — they belong to another club and can't be bought
                    let is_on_loan = player.contract.as_ref()
                        .map(|c| c.contract_type == crate::ContractType::Loan)
                        .unwrap_or(false);
                    if is_on_loan {
                        continue;
                    }

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
        let mut staff_events: Vec<(u32, u32, StaffEventType)> = Vec::new();

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

                let observe_chance = 60 + (judging_ability as i32 / 2);
                if IntegerUtils::random(0, 100) > observe_chance {
                    continue;
                }

                if let Some(scout_id) = assignment.scout_staff_id {
                    staff_events.push((club.id, scout_id, StaffEventType::PlayerScouted));
                }

                // Find matching players from OTHER clubs
                // Scouts look at position GROUP match, not just exact position
                // Value filter is intentionally loose — scouts assess talent broadly,
                // the shortlist/negotiation phase handles affordability
                let target_group = assignment.target_position.position_group();
                let matching: Vec<&PlayerSummary> = all_players
                    .iter()
                    .filter(|p| {
                        p.club_id != club.id
                            && p.position_group == target_group
                            && p.age >= assignment.preferred_age_min
                            && p.age <= assignment.preferred_age_max
                            && p.current_ability >= assignment.min_ability
                    })
                    .collect();

                if matching.is_empty() {
                    continue;
                }

                // Scouts observe 2-3 players per day (not just 1)
                let obs_per_day = 2 + (judging_ability as usize / 10); // 2-3

                for _obs_round in 0..obs_per_day.min(matching.len()) {
                    // 60% chance to re-observe a previously seen player (deepen knowledge)
                    // 40% chance to discover a new player
                    let already_observed_ids: Vec<u32> = assignment
                        .observations
                        .iter()
                        .map(|o| o.player_id)
                        .collect();

                    let target = if !already_observed_ids.is_empty()
                        && IntegerUtils::random(0, 100) < 60
                    {
                        // Prefer re-observing a known player
                        matching
                            .iter()
                            .find(|p| already_observed_ids.contains(&p.player_id))
                            .or_else(|| matching.first())
                            .unwrap()
                    } else {
                        // Discover new player
                        let new_players: Vec<&&PlayerSummary> = matching
                            .iter()
                            .filter(|p| !already_observed_ids.contains(&p.player_id))
                            .collect();
                        if !new_players.is_empty() {
                            let idx = (IntegerUtils::random(0, new_players.len() as i32) as usize)
                                .min(new_players.len() - 1);
                            new_players[idx]
                        } else {
                            let idx = (IntegerUtils::random(0, matching.len() as i32) as usize)
                                .min(matching.len() - 1);
                            matching[idx]
                        }
                    };

                    let existing_obs = assignment
                        .observations
                        .iter()
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

                    // Skip if we already queued an observation for this player this round
                    if observations.iter().any(|o| {
                        o.club_id == club.id
                            && o.assignment_id == assignment.id
                            && o.player_id == target.player_id
                    }) {
                        continue;
                    }

                    observations.push(ScoutingObservationResult {
                        club_id: club.id,
                        assignment_id: assignment.id,
                        player_id: target.player_id,
                        assessed_ability,
                        assessed_potential,
                        is_new,
                    });

                    // Generate report after just 1 observation (with lower confidence)
                    let final_obs_count = obs_count + 1;
                    let confidence = if final_obs_count == 1 {
                        0.4
                    } else {
                        1.0 - (1.0 / (final_obs_count as f32 + 1.0))
                    };

                    let recommendation =
                        if assessed_ability as i16 >= assignment.min_ability as i16 + 10
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
                        if assignment.reports_produced >= 2 {
                            assignment.completed = true;
                        }
                    }
                }
            }
        }

        // Push staff events for scouts
        for (club_id, staff_id, event_type) in staff_events {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == club_id) {
                for team in &mut club.teams.teams {
                    if let Some(staff) = team.staffs.staffs.iter_mut().find(|s| s.id == staff_id) {
                        staff.add_event(event_type);
                        break;
                    }
                }
            }
        }
    }

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
                    .filter(|r| r.estimated_value <= budget_alloc * 5.0)
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
                if results.iter().any(|r| r.club_id == club.id && r.request_id == request.id) {
                    continue;
                }

                let market_candidates: Vec<ShortlistCandidate> = country
                    .transfer_market
                    .get_available_listings()
                    .iter()
                    .filter(|l| l.club_id != club.id)
                    .filter_map(|l| {
                        Self::find_player_summary_in_country(country, l.player_id, date).and_then(
                            |p| {
                                if p.position_group == request.position.position_group()
                                    && p.current_ability >= request.min_ability
                                    && p.estimated_value <= request.budget_allocation * 5.0
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
                .unwrap_or_else(|| (club.finance.balance.balance.max(0) as f64) * 0.3);

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

                // Skip players on loan contracts — they belong to another club
                let is_on_loan = Self::find_player_in_country(country, player_id)
                    .and_then(|p| p.contract.as_ref())
                    .map(|c| c.contract_type == crate::ContractType::Loan)
                    .unwrap_or(false);
                if is_on_loan {
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
                    club.finance.balance.balance,
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
                            amount: crate::utils::FormattingUtils::round_fee(asking_price.amount * 0.1),
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

            // Loan-out candidates are handled by process_loan_out_listings()
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
                    amount: crate::utils::FormattingUtils::round_fee(action.offer.base_fee.amount * 1.2),
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
    /// - January window and negative balance rules bias toward loans
    fn determine_transfer_approach(
        rep_level: &ReputationLevel,
        budget: f64,
        estimated_fee: f64,
        request: Option<&TransferRequest>,
        country: &Country,
        player_id: u32,
        date: NaiveDate,
        buying_club_balance: i32,
    ) -> TransferApproach {
        let is_january = Self::is_january_window(date);

        // If the player is already loan-listed, pursue a loan
        if let Some(player) = Self::find_player_in_country(country, player_id) {
            let statuses = player.statuses.get();
            if statuses.contains(&PlayerStatusType::Loa) {
                return TransferApproach::Loan;
            }
        }

        // Reasons that always result in loan approach
        if let Some(req) = request {
            match req.reason {
                TransferNeedReason::DevelopmentSigning
                | TransferNeedReason::LoanToFillSquad
                | TransferNeedReason::InjuryCoverLoan
                | TransferNeedReason::OpportunisticLoanUpgrade
                | TransferNeedReason::SquadPadding => {
                    return TransferApproach::Loan;
                }
                TransferNeedReason::ExperiencedHead | TransferNeedReason::CheapReinforcement => {
                    // Prefer loan, but allow cheap buy if very affordable
                    if estimated_fee > 50_000.0 || buying_club_balance < 0 {
                        return TransferApproach::Loan;
                    }
                }
                _ => {}
            }
        }

        let is_critical = request
            .map(|r| r.priority == TransferNeedPriority::Critical)
            .unwrap_or(false);

        // January + Regional/Local/Amateur → always Loan
        if is_january
            && matches!(
                rep_level,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            )
        {
            return TransferApproach::Loan;
        }

        // January + National + non-Critical request → Loan
        if is_january && *rep_level == ReputationLevel::National && !is_critical {
            return TransferApproach::Loan;
        }

        // Negative balance + non-Elite → Loan
        if buying_club_balance < 0 && *rep_level != ReputationLevel::Elite {
            return TransferApproach::Loan;
        }

        // Can we even afford to buy?
        let affordability = if estimated_fee > 0.0 {
            budget / estimated_fee
        } else {
            10.0 // Free agent, always affordable
        };

        match rep_level {
            ReputationLevel::Elite => {
                if affordability >= 0.3 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::LoanWithOption
                }
            }
            ReputationLevel::Continental => {
                if affordability >= 0.4 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.15 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::National => {
                if affordability >= 0.6 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.25 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::Regional => {
                // Relaxed: threshold from 1.0→0.7 for buy, add LoanWithOption tier at 0.3
                if affordability >= 0.7 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.3 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            _ => {
                if affordability >= 1.5 && estimated_fee < 100_000.0 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::Loan
                }
            }
        }
    }

    // ============================================================
    // Step 6.5: Scan Loan Market — Small clubs proactively seek loans
    // ============================================================

    pub fn scan_loan_market(country: &mut Country, date: NaiveDate) {
        let is_january = Self::is_january_window(date);

        // Collect available loan listings (Pass 1 read)
        struct LoanListing {
            player_id: u32,
            club_id: u32,
            asking_price: f64,
            ability: u8,
            age: u8,
            position_group: PlayerFieldPositionGroup,
        }

        let mut loan_listings: Vec<LoanListing> = Vec::new();

        for listing in &country.transfer_market.listings {
            if listing.listing_type != TransferListingType::Loan {
                continue;
            }
            if listing.status != crate::transfers::market::TransferListingStatus::Available {
                continue;
            }
            if let Some(player) = Self::find_player_in_country(country, listing.player_id) {
                loan_listings.push(LoanListing {
                    player_id: listing.player_id,
                    club_id: listing.club_id,
                    asking_price: listing.asking_price.amount,
                    ability: player.player_attributes.current_ability,
                    age: player.age(date),
                    position_group: player.position().position_group(),
                });
            }
        }

        if loan_listings.is_empty() {
            return;
        }

        // Collect club scanning actions (Pass 1 read)
        struct LoanScanAction {
            club_id: u32,
            player_id: u32,
            selling_club_id: u32,
            offer_amount: f64,
        }

        let mut actions: Vec<LoanScanAction> = Vec::new();

        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }

            let team = &club.teams.teams[0];
            let rep_level = team.reputation.level();

            // Determine if club should scan
            let should_scan = match rep_level {
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur => {
                    true
                }
                ReputationLevel::National => {
                    is_january || club.finance.balance.balance < 0
                }
                ReputationLevel::Continental => {
                    is_january && club.finance.balance.balance < 0
                }
                ReputationLevel::Elite => false,
            };

            if !should_scan {
                continue;
            }

            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let balance = club.finance.balance.balance;
            let max_loan_fee = if balance < 0 {
                50_000.0
            } else {
                balance as f64 * 0.20
            };

            let max_scans: usize = match rep_level {
                ReputationLevel::Local | ReputationLevel::Amateur => 4,
                ReputationLevel::Regional => 3,
                ReputationLevel::National => 2,
                _ => 1,
            };
            let mut scans_this_club = 0usize;

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

            // Check unfulfilled transfer requests first
            let unfulfilled: Vec<&TransferRequest> = plan
                .transfer_requests
                .iter()
                .filter(|r| {
                    r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                })
                .collect();

            for request in &unfulfilled {
                if scans_this_club >= max_scans {
                    break;
                }

                // Relaxed thresholds: min_ability - 5, age_max + 3
                let relaxed_min = request.min_ability.saturating_sub(5);
                let relaxed_age_max = request.preferred_age_max.saturating_add(3);

                if let Some(best) = loan_listings
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && l.position_group == request.position.position_group()
                            && l.ability >= relaxed_min
                            && l.age <= relaxed_age_max
                            && l.age >= request.preferred_age_min
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                    })
                    .max_by_key(|l| l.ability)
                {
                    actions.push(LoanScanAction {
                        club_id: club.id,
                        player_id: best.player_id,
                        selling_club_id: best.club_id,
                        offer_amount: crate::utils::FormattingUtils::round_fee(best.asking_price * 0.8),
                    });
                    scans_this_club += 1;
                }
            }

            // Opportunistic scan: small clubs always look for deals, not just in January
            let is_small_club = matches!(
                rep_level,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            );

            // Small clubs scan for ANY decent player they can loan cheaply
            if is_small_club && scans_this_club < max_scans {
                // Look for players above squad average — a loan upgrade opportunity
                let mut opps: Vec<&LoanListing> = loan_listings
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && l.ability >= avg_ability.saturating_sub(5)
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions
                                .iter()
                                .any(|a| a.club_id == club.id && a.player_id == l.player_id)
                    })
                    .collect();
                opps.sort_by(|a, b| b.ability.cmp(&a.ability));

                for opp in opps.iter().take(max_scans - scans_this_club) {
                    actions.push(LoanScanAction {
                        club_id: club.id,
                        player_id: opp.player_id,
                        selling_club_id: opp.club_id,
                        offer_amount: crate::utils::FormattingUtils::round_fee(opp.asking_price * 0.8),
                    });
                    scans_this_club += 1;
                }
            }

            // January extra: even National clubs look for opportunistic loans
            if is_january && scans_this_club < max_scans && !is_small_club {
                if let Some(opp) = loan_listings
                    .iter()
                    .filter(|l| {
                        l.club_id != club.id
                            && l.ability >= avg_ability.saturating_sub(8)
                            && l.asking_price * 0.8 <= max_loan_fee
                            && !country
                                .transfer_market
                                .has_active_negotiation_for(l.player_id, club.id)
                            && !actions
                                .iter()
                                .any(|a| a.club_id == club.id && a.player_id == l.player_id)
                    })
                    .max_by_key(|l| l.ability)
                {
                    actions.push(LoanScanAction {
                        club_id: club.id,
                        player_id: opp.player_id,
                        selling_club_id: opp.club_id,
                        offer_amount: crate::utils::FormattingUtils::round_fee(opp.asking_price * 0.8),
                    });
                }
            }
        }

        // Pass 2: Start loan negotiations
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) =
                Self::get_player_negotiation_data(country, action.player_id, date);

            let offer = crate::transfers::offer::TransferOffer {
                base_fee: CurrencyValue {
                    amount: action.offer_amount,
                    currency: Currency::Usd,
                },
                clauses: Vec::new(),
                salary_contribution: None,
                contract_length: Some(1),
                offering_club_id: action.club_id,
                offered_date: date,
            };

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.club_id,
                offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = true;
                    negotiation.is_unsolicited = false;
                }

                debug!(
                    "Loan scan: Club {} started loan negotiation for player {}",
                    action.club_id, action.player_id
                );
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
    // Step 2.5: Generate Staff Recommendations
    // ============================================================

    pub fn generate_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate(date) {
            return;
        }

        let is_january = Self::is_january_window(date);
        let price_level = country.settings.pricing.price_level;

        // Pass 1: Build player snapshots across all clubs
        #[allow(dead_code)]
        struct PlayerSnapshot {
            id: u32,
            club_id: u32,
            position: PlayerPositionType,
            position_group: PlayerFieldPositionGroup,
            ability: u8,
            potential: u8,
            age: u8,
            estimated_value: f64,
            contract_months_remaining: u32,
            club_in_debt: bool,
            parent_club_reputation: ReputationLevel,
            is_loan_listed: bool,
        }

        let mut all_snapshots: Vec<PlayerSnapshot> = Vec::new();

        for club in &country.clubs {
            let club_in_debt = club.finance.balance.balance < 0;
            let rep_level = club
                .teams
                .teams
                .first()
                .map(|t| t.reputation.level())
                .unwrap_or(ReputationLevel::Amateur);

            for team in &club.teams.teams {
                for player in &team.players.players {
                    let value = PlayerValuationCalculator::calculate_value_with_price_level(
                        player,
                        date,
                        price_level,
                    );
                    let contract_months = player
                        .contract
                        .as_ref()
                        .map(|c| {
                            let days = (c.expiration - date).num_days().max(0) as u32;
                            days / 30
                        })
                        .unwrap_or(0);

                    let statuses = player.statuses.get();

                    all_snapshots.push(PlayerSnapshot {
                        id: player.id,
                        club_id: club.id,
                        position: player.position(),
                        position_group: player.position().position_group(),
                        ability: player.player_attributes.current_ability,
                        potential: player.player_attributes.potential_ability,
                        age: player.age(date),
                        estimated_value: value.amount,
                        contract_months_remaining: contract_months,
                        club_in_debt,
                        parent_club_reputation: rep_level.clone(),
                        is_loan_listed: statuses.contains(&PlayerStatusType::Loa),
                    });
                }
            }
        }

        // Collect recommendations per club
        struct RecommendationAction {
            club_id: u32,
            recommendation: StaffRecommendation,
        }

        let mut actions: Vec<RecommendationAction> = Vec::new();

        for club in &country.clubs {
            if club.teams.teams.is_empty() {
                continue;
            }
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            // Cap: 6 recommendations per club per window
            if plan.staff_recommendations.len() >= 6 {
                continue;
            }

            let team = &club.teams.teams[0];
            let resolved = StaffResolver::resolve(&team.staffs);

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

            let club_rep = team.reputation.level();

            let already_recommended: Vec<u32> = plan
                .staff_recommendations
                .iter()
                .map(|r| r.player_id)
                .collect();

            // ── Scout network recommendations ──
            for scout in &resolved.scouts {
                let judging = scout.staff_attributes.knowledge.judging_player_ability;
                let judging_pot = scout.staff_attributes.knowledge.judging_player_potential;

                // Discovery chance: 10 + (judging_ability * 3) percent
                let discovery_chance = 10 + (judging as i32 * 3);
                if IntegerUtils::random(0, 100) > discovery_chance {
                    continue;
                }

                // Filter candidates from other clubs
                let candidates: Vec<&PlayerSnapshot> = all_snapshots
                    .iter()
                    .filter(|p| {
                        p.club_id != club.id
                            && p.ability >= avg_ability.saturating_sub(10)
                            && p.ability <= avg_ability + (judging / 2)
                            && !already_recommended.contains(&p.id)
                            && !actions
                                .iter()
                                .any(|a| a.club_id == club.id && a.recommendation.player_id == p.id)
                    })
                    .collect();

                if candidates.is_empty() {
                    continue;
                }

                // Score candidates
                let mut best_score = 0.0f32;
                let mut best_candidate: Option<&PlayerSnapshot> = None;

                for cand in &candidates {
                    let mut score: f32 = 0.0;

                    // Expiring contract
                    if cand.contract_months_remaining <= 6 {
                        score += 3.0;
                    } else if cand.contract_months_remaining <= 12 {
                        score += 1.5;
                    }

                    // Club in debt
                    if cand.club_in_debt {
                        score += 2.0;
                    }

                    // High potential gap
                    if cand.potential > cand.ability + 15 {
                        score += 2.5;
                    } else if cand.potential > cand.ability + 8 {
                        score += 1.5;
                    }

                    // Lower-rep club
                    if Self::rep_level_value(&cand.parent_club_reputation)
                        < Self::rep_level_value(&club_rep)
                    {
                        score += 1.0;
                    }

                    // Loan-listed
                    if cand.is_loan_listed {
                        score += if is_january { 2.0 } else { 1.0 };
                    }

                    // Ability fit
                    if cand.ability >= avg_ability.saturating_sub(5) {
                        score += 1.0;
                    }

                    if score > best_score {
                        best_score = score;
                        best_candidate = Some(cand);
                    }
                }

                if let Some(cand) = best_candidate {
                    // Assess with error based on judging skill
                    let ability_error = (20i16 - judging as i16).max(1) as i32;
                    let potential_error = (20i16 - judging_pot as i16).max(1) as i32;

                    let assessed_ability = (cand.ability as i32
                        + IntegerUtils::random(-ability_error, ability_error))
                    .clamp(1, 100) as u8;
                    let assessed_potential = (cand.potential as i32
                        + IntegerUtils::random(-potential_error, potential_error))
                    .clamp(1, 100) as u8;

                    let confidence = (0.3 + (judging as f32 * 0.035)).min(0.95);

                    let rec_type = if cand.contract_months_remaining <= 6 {
                        RecommendationType::ExpiringContract
                    } else if cand.club_in_debt {
                        RecommendationType::FinancialDistress
                    } else if cand.potential > cand.ability + 15 && cand.age <= 22 {
                        RecommendationType::HiddenGem
                    } else if cand.is_loan_listed {
                        RecommendationType::LoanOpportunity
                    } else {
                        RecommendationType::ReadyForStepUp
                    };

                    actions.push(RecommendationAction {
                        club_id: club.id,
                        recommendation: StaffRecommendation {
                            player_id: cand.id,
                            recommender_staff_id: scout.id,
                            source: RecommendationSource::ScoutNetwork,
                            recommendation_type: rec_type,
                            assessed_ability,
                            assessed_potential,
                            confidence,
                            estimated_fee: cand.estimated_value,
                            date_recommended: date,
                        },
                    });
                }
            }

            // ── DoF bargain identification ──
            if let Some(dof) = resolved.director_of_football {
                let judging = dof.staff_attributes.knowledge.judging_player_ability;
                let judging_pot = dof.staff_attributes.knowledge.judging_player_potential;
                let dof_chance = 40 + (judging as i32 * 3);

                if IntegerUtils::random(0, 100) <= dof_chance {
                    // Look for expiring contracts with ability >= avg-5
                    let dof_candidates: Vec<&PlayerSnapshot> = all_snapshots
                        .iter()
                        .filter(|p| {
                            p.club_id != club.id
                                && p.contract_months_remaining <= 6
                                && p.ability >= avg_ability.saturating_sub(5)
                                && !already_recommended.contains(&p.id)
                                && !actions.iter().any(|a| {
                                    a.club_id == club.id && a.recommendation.player_id == p.id
                                })
                        })
                        .collect();

                    if let Some(best) = dof_candidates.iter().max_by_key(|p| p.ability) {
                        let ability_error = (20i16 - judging as i16).max(1) as i32;
                        let potential_error = (20i16 - judging_pot as i16).max(1) as i32;

                        let assessed_ability = (best.ability as i32
                            + IntegerUtils::random(-ability_error, ability_error))
                        .clamp(1, 100) as u8;
                        let assessed_potential = (best.potential as i32
                            + IntegerUtils::random(-potential_error, potential_error))
                        .clamp(1, 100) as u8;

                        let confidence = (0.4 + (judging as f32 * 0.035)).min(0.95);

                        actions.push(RecommendationAction {
                            club_id: club.id,
                            recommendation: StaffRecommendation {
                                player_id: best.id,
                                recommender_staff_id: dof.id,
                                source: RecommendationSource::DirectorOfFootball,
                                recommendation_type: RecommendationType::ExpiringContract,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_fee: best.estimated_value,
                                date_recommended: date,
                            },
                        });
                    }
                }
            }

            // ── Small club staff: aggressive loan/bargain hunting ──
            // Small clubs rely on their staff to find cheap deals, loans,
            // free agents, and surplus players from bigger clubs.
            // Even a head coach at a small club knows what the squad needs.
            let is_small_club = matches!(
                club_rep,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            );
            let is_mid_club = club_rep == ReputationLevel::National;

            if is_small_club || is_mid_club {
                let rec_cap = if is_small_club { 10 } else { 8 };
                let current_recs = plan.staff_recommendations.len()
                    + actions.iter().filter(|a| a.club_id == club.id).count();

                if current_recs < rec_cap {
                    let remaining = rec_cap - current_recs;

                    // Coach recommends players available on loan
                    let head_coach = team.staffs.head_coach();
                    let coach_id = head_coach.id;
                    let coach_judging =
                        head_coach.staff_attributes.knowledge.judging_player_ability;
                    let coach_judging_pot =
                        head_coach.staff_attributes.knowledge.judging_player_potential;

                    // ── Cheap loan targets (loan-listed players the club could afford) ──
                    let mut loan_targets: Vec<&PlayerSnapshot> = all_snapshots
                        .iter()
                        .filter(|p| {
                            p.club_id != club.id
                                && p.is_loan_listed
                                && p.ability >= avg_ability.saturating_sub(8)
                                && !already_recommended.contains(&p.id)
                                && !actions.iter().any(|a| {
                                    a.club_id == club.id && a.recommendation.player_id == p.id
                                })
                        })
                        .collect();
                    loan_targets.sort_by(|a, b| b.ability.cmp(&a.ability));

                    for target in loan_targets.iter().take(remaining.min(3)) {
                        let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                        let potential_error = (20i16 - coach_judging_pot as i16).max(1) as i32;

                        let assessed_ability = (target.ability as i32
                            + IntegerUtils::random(-ability_error, ability_error))
                        .clamp(1, 100) as u8;
                        let assessed_potential = (target.potential as i32
                            + IntegerUtils::random(-potential_error, potential_error))
                        .clamp(1, 100) as u8;

                        let rec_type = if target.ability > avg_ability + 5 {
                            RecommendationType::BigClubSurplus
                        } else if target.age >= 28 {
                            RecommendationType::ExperiencedLoanMentor
                        } else {
                            RecommendationType::CheapLoanAvailable
                        };

                        let confidence = (0.4 + (coach_judging as f32 * 0.03)).min(0.9);

                        actions.push(RecommendationAction {
                            club_id: club.id,
                            recommendation: StaffRecommendation {
                                player_id: target.id,
                                recommender_staff_id: coach_id,
                                source: RecommendationSource::HeadCoach,
                                recommendation_type: rec_type,
                                assessed_ability,
                                assessed_potential,
                                confidence,
                                estimated_fee: target.estimated_value * 0.1, // loan fee
                                date_recommended: date,
                            },
                        });
                    }

                    let current_recs_after_loans = plan.staff_recommendations.len()
                        + actions.iter().filter(|a| a.club_id == club.id).count();
                    let remaining_after_loans = rec_cap.saturating_sub(current_recs_after_loans);

                    // ── Free agent bargains (expiring contracts) ──
                    if remaining_after_loans > 0 {
                        let mut free_targets: Vec<&PlayerSnapshot> = all_snapshots
                            .iter()
                            .filter(|p| {
                                p.club_id != club.id
                                    && p.contract_months_remaining <= 6
                                    && p.ability >= avg_ability.saturating_sub(10)
                                    && !already_recommended.contains(&p.id)
                                    && !actions.iter().any(|a| {
                                        a.club_id == club.id
                                            && a.recommendation.player_id == p.id
                                    })
                            })
                            .collect();
                        free_targets.sort_by(|a, b| b.ability.cmp(&a.ability));

                        for target in free_targets.iter().take(remaining_after_loans.min(2)) {
                            let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                            let potential_error =
                                (20i16 - coach_judging_pot as i16).max(1) as i32;

                            let assessed_ability = (target.ability as i32
                                + IntegerUtils::random(-ability_error, ability_error))
                            .clamp(1, 100) as u8;
                            let assessed_potential = (target.potential as i32
                                + IntegerUtils::random(-potential_error, potential_error))
                            .clamp(1, 100) as u8;

                            let confidence = (0.5 + (coach_judging as f32 * 0.03)).min(0.9);

                            actions.push(RecommendationAction {
                                club_id: club.id,
                                recommendation: StaffRecommendation {
                                    player_id: target.id,
                                    recommender_staff_id: coach_id,
                                    source: RecommendationSource::HeadCoach,
                                    recommendation_type: RecommendationType::FreeAgentBargain,
                                    assessed_ability,
                                    assessed_potential,
                                    confidence,
                                    estimated_fee: 0.0, // free agent
                                    date_recommended: date,
                                },
                            });
                        }
                    }

                    let current_recs_after_free = plan.staff_recommendations.len()
                        + actions.iter().filter(|a| a.club_id == club.id).count();
                    let remaining_after_free = rec_cap.saturating_sub(current_recs_after_free);

                    // ── Players wanting game time from bigger clubs ──
                    // Young players at bigger clubs who aren't loan-listed yet but
                    // are below their club's average — they'd benefit from a loan
                    if remaining_after_free > 0 && is_small_club {
                        let mut game_time_seekers: Vec<&PlayerSnapshot> = all_snapshots
                            .iter()
                            .filter(|p| {
                                p.club_id != club.id
                                    && p.age <= 23
                                    && p.potential > p.ability + 5
                                    && p.ability >= avg_ability.saturating_sub(5)
                                    && Self::rep_level_value(&p.parent_club_reputation)
                                        > Self::rep_level_value(&club_rep)
                                    && !p.is_loan_listed
                                    && !already_recommended.contains(&p.id)
                                    && !actions.iter().any(|a| {
                                        a.club_id == club.id
                                            && a.recommendation.player_id == p.id
                                    })
                            })
                            .collect();
                        game_time_seekers.sort_by(|a, b| b.potential.cmp(&a.potential));

                        for target in game_time_seekers.iter().take(remaining_after_free.min(2))
                        {
                            let ability_error = (20i16 - coach_judging as i16).max(1) as i32;
                            let potential_error =
                                (20i16 - coach_judging_pot as i16).max(1) as i32;

                            let assessed_ability = (target.ability as i32
                                + IntegerUtils::random(-ability_error, ability_error))
                            .clamp(1, 100) as u8;
                            let assessed_potential = (target.potential as i32
                                + IntegerUtils::random(-potential_error, potential_error))
                            .clamp(1, 100) as u8;

                            let confidence = (0.3 + (coach_judging as f32 * 0.025)).min(0.8);

                            actions.push(RecommendationAction {
                                club_id: club.id,
                                recommendation: StaffRecommendation {
                                    player_id: target.id,
                                    recommender_staff_id: coach_id,
                                    source: RecommendationSource::HeadCoach,
                                    recommendation_type: RecommendationType::GameTimeSeeker,
                                    assessed_ability,
                                    assessed_potential,
                                    confidence,
                                    estimated_fee: target.estimated_value * 0.05, // loan fee
                                    date_recommended: date,
                                },
                            });
                        }
                    }
                }
            }
        }

        // Pass 2: Push recommendations into club transfer plans
        // Small clubs get higher cap
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let rep = club
                    .teams
                    .teams
                    .first()
                    .map(|t| t.reputation.level())
                    .unwrap_or(ReputationLevel::Amateur);
                let cap = match rep {
                    ReputationLevel::Regional
                    | ReputationLevel::Local
                    | ReputationLevel::Amateur => 10,
                    ReputationLevel::National => 8,
                    _ => 6,
                };
                if club.transfer_plan.staff_recommendations.len() < cap {
                    club.transfer_plan
                        .staff_recommendations
                        .push(action.recommendation);
                }
            }
        }
    }

    // ============================================================
    // Step 2.75: Process Staff Recommendations
    // ============================================================

    pub fn process_staff_recommendations(country: &mut Country, date: NaiveDate) {
        // Only runs weekly (same schedule as should_evaluate)
        if !Self::should_evaluate(date) {
            return;
        }

        struct RecommendationProcessAction {
            club_id: u32,
            kind: RecommendationProcessKind,
        }

        enum RecommendationProcessKind {
            AddToShortlist {
                shortlist_request_id: u32,
                candidate: ShortlistCandidate,
            },
            CreateRequest {
                request: TransferRequest,
            },
        }

        let mut actions: Vec<RecommendationProcessAction> = Vec::new();
        let seven_days_ago = date - chrono::Duration::days(7);

        for club in &country.clubs {
            let plan = &club.transfer_plan;
            if !plan.initialized {
                continue;
            }

            let recent_recs: Vec<&StaffRecommendation> = plan
                .staff_recommendations
                .iter()
                .filter(|r| r.date_recommended >= seven_days_ago)
                .collect();

            for rec in &recent_recs {
                // Determine player's position group
                let player_pos_group =
                    if let Some(player) = Self::find_player_in_country(country, rec.player_id) {
                        player.position().position_group()
                    } else {
                        continue;
                    };

                // Check if an existing unfulfilled request covers the same position group
                let matching_request = plan.transfer_requests.iter().find(|r| {
                    r.position.position_group() == player_pos_group
                        && r.status != TransferRequestStatus::Fulfilled
                        && r.status != TransferRequestStatus::Abandoned
                });

                if let Some(req) = matching_request {
                    // Find the shortlist for this request
                    let has_shortlist = plan
                        .shortlists
                        .iter()
                        .any(|s| s.transfer_request_id == req.id);

                    if has_shortlist {
                        // Add as candidate to existing shortlist
                        let already_in = plan.shortlists.iter().any(|s| {
                            s.transfer_request_id == req.id
                                && s.candidates.iter().any(|c| c.player_id == rec.player_id)
                        });

                        if !already_in {
                            actions.push(RecommendationProcessAction {
                                club_id: club.id,
                                kind: RecommendationProcessKind::AddToShortlist {
                                    shortlist_request_id: req.id,
                                    candidate: ShortlistCandidate {
                                        player_id: rec.player_id,
                                        score: rec.assessed_ability as f32 / 100.0
                                            + rec.confidence * 0.1,
                                        estimated_fee: rec.estimated_fee,
                                        status: ShortlistCandidateStatus::Available,
                                    },
                                },
                            });
                        }
                    }
                } else if rec.confidence >= 0.6 && rec.assessed_ability >= 50 {
                    // No existing request — create a new one
                    let player_position =
                        if let Some(player) = Self::find_player_in_country(country, rec.player_id)
                        {
                            player.position()
                        } else {
                            continue;
                        };

                    // Check we don't already have too many requests
                    let active_requests = plan
                        .transfer_requests
                        .iter()
                        .filter(|r| {
                            r.status != TransferRequestStatus::Fulfilled
                                && r.status != TransferRequestStatus::Abandoned
                        })
                        .count();

                    if active_requests >= 8 {
                        continue;
                    }

                    // Allocate 15% of available budget
                    let available_budget = plan.available_budget();
                    let alloc = available_budget * 0.15;

                    if alloc <= 0.0 {
                        continue;
                    }

                    let next_id = plan.next_request_id + actions
                        .iter()
                        .filter(|a| {
                            a.club_id == club.id
                                && matches!(a.kind, RecommendationProcessKind::CreateRequest { .. })
                        })
                        .count() as u32;

                    actions.push(RecommendationProcessAction {
                        club_id: club.id,
                        kind: RecommendationProcessKind::CreateRequest {
                            request: TransferRequest::new(
                                next_id,
                                player_position,
                                TransferNeedPriority::Optional,
                                TransferNeedReason::StaffRecommendation,
                                rec.assessed_ability.saturating_sub(5),
                                rec.assessed_ability,
                                alloc,
                            ),
                        },
                    });
                }
            }
        }

        // Pass 2: Apply actions
        for action in actions {
            if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
                let plan = &mut club.transfer_plan;

                match action.kind {
                    RecommendationProcessKind::AddToShortlist {
                        shortlist_request_id,
                        candidate,
                    } => {
                        if let Some(shortlist) = plan
                            .shortlists
                            .iter_mut()
                            .find(|s| s.transfer_request_id == shortlist_request_id)
                        {
                            shortlist.candidates.push(candidate);
                        }
                    }
                    RecommendationProcessKind::CreateRequest { request } => {
                        let req_id = request.id;
                        if req_id >= plan.next_request_id {
                            plan.next_request_id = req_id + 1;
                        }
                        plan.transfer_requests.push(request);
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
        date: NaiveDate,
    ) -> Option<PlayerSummary> {
        for club in &country.clubs {
            for team in &club.teams.teams {
                if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                    return Some(PlayerSummary {
                        player_id: player.id,
                        club_id: club.id,
                        position: player.position(),
                        position_group: player.position().position_group(),
                        current_ability: player.player_attributes.current_ability,
                        potential_ability: player.player_attributes.potential_ability,
                        age: player.age(date),
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
            amount: crate::utils::FormattingUtils::round_fee(base_value.amount * multiplier),
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

    fn rep_level_value(level: &ReputationLevel) -> u8 {
        match level {
            ReputationLevel::Elite => 5,
            ReputationLevel::Continental => 4,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            ReputationLevel::Local => 1,
            ReputationLevel::Amateur => 0,
        }
    }
}
