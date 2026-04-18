use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

use crate::transfers::pipeline::{
    ClubTransferPlan, LoanOutCandidate, LoanOutReason, LoanOutStatus,
    TransferNeedPriority, TransferNeedReason, TransferRequest,
    TransferRequestStatus,
};
use crate::transfers::pipeline::processor::{PipelineProcessor, SquadPlayerInfo};
use crate::transfers::TransferWindowManager;
use crate::{
    Club, ClubPhilosophy, Country, MatchTacticType, Person, Player,
    PlayerFieldPositionGroup, PlayerPlanRole, PlayerPositionType, ReputationLevel,
    TacticsSelector, TACTICS_POSITIONS,
};

struct SquadEvaluation {
    club_id: u32,
    requests: Vec<TransferRequest>,
    loan_outs: Vec<LoanOutCandidate>,
    total_budget: f64,
    max_concurrent: u32,
}

impl PipelineProcessor {
    // ============================================================
    // Step 2: Squad Evaluation - Coach-driven, formation-based
    // ============================================================

    pub fn evaluate_squads(country: &mut Country, date: NaiveDate) {
        let is_window_start = Self::is_window_start(date);
        let should_evaluate = is_window_start || Self::should_evaluate(date);
        let window_mgr = TransferWindowManager::new();
        let current_window = window_mgr.current_window_dates(country.id, date);

        // Pass 1: Collect evaluations (immutable reads)
        let mut evaluations: Vec<SquadEvaluation> = Vec::new();

        for club in &country.clubs {
            let needs_eval = should_evaluate
                || !club.transfer_plan.initialized
                || Self::all_shortlists_exhausted(&club.transfer_plan);

            if !needs_eval {
                continue;
            }

            let eval = Self::evaluate_single_club(club, date, current_window);
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

                // Only add requests that don't duplicate an existing unfulfilled request
                // for the same position group. Without this, each weekly re-evaluation
                // adds duplicate requests (e.g. "need GK") that all get acted on independently,
                // causing clubs to loan 10+ players for the same position.
                let new_requests: Vec<_> = eval.requests.into_iter().filter(|new_req| {
                    !plan.transfer_requests.iter().any(|existing| {
                        existing.position.position_group() == new_req.position.position_group()
                            && existing.status != TransferRequestStatus::Fulfilled
                            && existing.status != TransferRequestStatus::Abandoned
                    })
                }).collect();
                plan.transfer_requests.extend(new_requests);
                // Deduplicate loan-out candidates — don't re-add players already in the list
                for candidate in eval.loan_outs {
                    if !plan.loan_out_candidates.iter().any(|existing| existing.player_id == candidate.player_id) {
                        plan.loan_out_candidates.push(candidate);
                    }
                }
                plan.last_evaluation_date = Some(date);
                plan.initialized = true;
            }
        }
    }

    fn all_shortlists_exhausted(plan: &ClubTransferPlan) -> bool {
        if plan.shortlists.is_empty() {
            return false;
        }
        plan.shortlists.iter().all(|s| s.all_exhausted())
    }

    /// Core squad evaluation: the head coach analyzes the squad based on their preferred
    /// formation and identifies tactical gaps.
    fn evaluate_single_club(club: &Club, date: NaiveDate, current_window: Option<(NaiveDate, NaiveDate)>) -> SquadEvaluation {
        let mut requests = Vec::new();
        let mut loan_outs = Vec::new();
        let mut next_id = club.transfer_plan.next_request_id;

        // Calculate budget. Clubs in FFP breach have half their buying
        // power until the losses unwind — a soft equivalent of the
        // real-world transfer ban / spending cap.
        let raw_budget = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or_else(|| (club.finance.balance.balance.max(0) as f64) * 0.3);
        let budget = if club.finance.is_ffp_breach(date) {
            raw_budget * 0.5
        } else {
            raw_budget
        };

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

        // For Elite/Continental clubs, use top-11 (starter) average to avoid
        // dragging the threshold down with weak youth/reserve players.
        // This prevents top clubs from pursuing mediocre transfer targets.
        let avg_ability: u8 = if !squad.is_empty() {
            let mut abilities: Vec<u8> = squad.iter().map(|p| p.current_ability).collect();
            abilities.sort_unstable_by(|a, b| b.cmp(a));

            let count = match rep_level {
                ReputationLevel::Elite | ReputationLevel::Continental => {
                    abilities.len().min(11) // top-11 starter average
                }
                ReputationLevel::National => {
                    abilities.len().min(16) // top-16 average
                }
                _ => abilities.len(), // full squad average for smaller clubs
            };

            let total: u32 = abilities[..count].iter().map(|&a| a as u32).sum();
            (total / count as u32) as u8
        } else {
            50
        };

        // ──────────────────────────────────────────────────────────
        // Philosophy-driven parameters
        // ──────────────────────────────────────────────────────────

        // Philosophy shapes transfer priorities: what age to target,
        // how much to spend on youth vs proven players, loan appetite.
        let philosophy = &club.philosophy;

        // Age preferences: DevelopAndSell targets young players,
        // SignToCompete targets prime-age proven performers
        let (_preferred_age_min, _preferred_age_max, youth_age_max) = match philosophy {
            ClubPhilosophy::DevelopAndSell => (17u8, 26, 21),
            ClubPhilosophy::SignToCompete => (23, 32, 19),
            ClubPhilosophy::LoanFocused => (19, 28, 22),
            ClubPhilosophy::Balanced => (19, 30, 21),
        };

        // Ability threshold adjustment: youth-focused clubs accept lower CA
        // because they invest in potential; compete-now clubs need immediate quality
        let ability_tolerance: i16 = match philosophy {
            ClubPhilosophy::DevelopAndSell => 25,   // accept CA 25 below avg
            ClubPhilosophy::SignToCompete => 5,     // only near or above avg
            ClubPhilosophy::LoanFocused => 15,
            ClubPhilosophy::Balanced => 15,
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

        // Quality issues: a formation slot's best player is well below squad level.
        // But only flag positions where the group genuinely lacks quality starters —
        // a weak 4th-choice defender is normal, not a reason to buy another one.
        //
        // Threshold is position-group aware: goalkeepers naturally score
        // lower on the raw CA scale (fewer outfield-style attributes feed
        // their rating), so a -15 gate against a team's top-11 mean fired
        // on nearly every mid-tier starting keeper and generated a "need
        // an upgrade" request immediately. The wider -25 gate for GKs
        // mirrors how clubs actually look at the position: the starter
        // stays unless he's clearly a weak link, not just "below the
        // striker's numbers on paper".
        let quality_gap = |group: PlayerFieldPositionGroup| -> i16 {
            match group {
                PlayerFieldPositionGroup::Goalkeeper => 25,
                _ => 15,
            }
        };

        let quality_issues: Vec<_> = position_coverage
            .iter()
            .filter(|(pos, player, quality)| {
                let group = pos.position_group();
                let gap = quality_gap(group);
                if player.is_none() || (*quality as i16) >= avg_ability as i16 - gap {
                    return false;
                }

                // How many formation slots need this position group?
                let formation_need = formation_positions
                    .iter()
                    .filter(|p| p.position_group() == group)
                    .count();

                // How many squad players in this group are already good enough?
                let good_players = squad
                    .iter()
                    .filter(|p| p.primary_position.position_group() == group)
                    .filter(|p| (p.current_ability as i16) >= avg_ability as i16 - gap)
                    .count();

                // If we have enough good players to fill all formation slots,
                // the flagged player is just a backup — no upgrade needed.
                good_players < formation_need
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

            let min_ca = (avg_ability as i16 - ability_tolerance).max(1) as u8;
            requests.push(TransferRequest::new(
                next_id,
                *pos,
                TransferNeedPriority::Critical,
                TransferNeedReason::FormationGap,
                min_ca,
                avg_ability,
                alloc,
            ));
            next_id += 1;
            budget_used += alloc;
        }

        // Quality issues - IMPORTANT: player is significantly below squad level
        // Deduplicate within the same position group — buying two CBs when one
        // is enough causes the second to sit on 0 apps and get dumped at a loss.
        let mut quality_groups_handled: Vec<PlayerFieldPositionGroup> = Vec::new();
        for (pos, _, _) in &quality_issues {
            let group = pos.position_group();
            if quality_groups_handled.contains(&group) {
                continue;
            }
            // Don't duplicate a FormationGap request for the same group
            if requests.iter().any(|r| r.position.position_group() == group) {
                continue;
            }
            quality_groups_handled.push(group);

            let alloc = budget_per_need.min(available_budget - budget_used);
            if alloc <= 0.0 {
                break;
            }

            let min_ca = (avg_ability as i16 - ability_tolerance / 2).max(1) as u8;
            requests.push(TransferRequest::new(
                next_id,
                *pos,
                TransferNeedPriority::Important,
                TransferNeedReason::QualityUpgrade,
                min_ca,
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

            let min_ca = (avg_ability as i16 - ability_tolerance).max(1) as u8;
            requests.push(TransferRequest::new(
                next_id,
                pos,
                TransferNeedPriority::Optional,
                TransferNeedReason::DepthCover,
                min_ca,
                avg_ability.saturating_sub(5),
                alloc,
            ));
            next_id += 1;
            budget_used += alloc;
        }

        // ──────────────────────────────────────────────────────────
        // STEP 4: Succession planning for aging key players
        // ──────────────────────────────────────────────────────────

        // Succession planning: proactive clubs replace aging stars before decline.
        // SignToCompete and DevelopAndSell clubs always plan; Balanced only at top tiers.
        let does_succession = matches!(philosophy,
            ClubPhilosophy::SignToCompete | ClubPhilosophy::DevelopAndSell)
            || matches!(rep_level, ReputationLevel::Elite | ReputationLevel::Continental);
        if does_succession {
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
        // STEP 4b: Youth development signings
        // Elite/Continental clubs actively seek young prospects with
        // high potential — even if current ability is well below squad level.
        // Like Juventus loaning a 19yo from Serie B who could become world class.
        // ──────────────────────────────────────────────────────────

        // Youth development signings: philosophy-driven.
        // DevelopAndSell clubs aggressively sign young prospects.
        // SignToCompete clubs rarely invest in youth (they buy ready-made).
        // LoanFocused clubs borrow young players instead.
        let wants_youth = match philosophy {
            ClubPhilosophy::DevelopAndSell => true,
            ClubPhilosophy::Balanced => matches!(rep_level, ReputationLevel::Elite | ReputationLevel::Continental | ReputationLevel::National),
            ClubPhilosophy::LoanFocused => false, // they borrow, not buy
            ClubPhilosophy::SignToCompete => false, // they buy proven players
        };
        if wants_youth {
            let position_groups = [
                PlayerFieldPositionGroup::Defender,
                PlayerFieldPositionGroup::Midfielder,
                PlayerFieldPositionGroup::Forward,
            ];

            let max_youth_requests = match philosophy {
                ClubPhilosophy::DevelopAndSell => 4, // aggressive youth policy
                _ => match rep_level {
                    ReputationLevel::Elite => 3,
                    ReputationLevel::Continental => 2,
                    _ => 1,
                },
            };
            let mut youth_requests = 0u32;

            for group in &position_groups {
                if youth_requests >= max_youth_requests {
                    break;
                }

                // Count young players in this position group
                let young_in_group = squad
                    .iter()
                    .filter(|p| p.primary_position.position_group() == *group && p.age <= youth_age_max)
                    .count();

                // DevelopAndSell wants more youth pipeline depth
                let min_young = if matches!(philosophy, ClubPhilosophy::DevelopAndSell) { 3 } else { 2 };
                if young_in_group < min_young {
                    let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
                    if alloc <= 0.0 {
                        break;
                    }

                    let pos = match group {
                        PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
                        PlayerFieldPositionGroup::Midfielder => PlayerPositionType::MidfielderCenter,
                        PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
                        _ => continue,
                    };

                    // Don't duplicate if we already have a request for this position group
                    if !requests.iter().any(|r| r.position.position_group() == *group) {
                        requests.push(TransferRequest::new(
                            next_id,
                            pos,
                            TransferNeedPriority::Optional,
                            TransferNeedReason::DevelopmentSigning,
                            // Low current ability floor — we care about potential, not now
                            avg_ability.saturating_sub(40),
                            avg_ability.saturating_sub(15),
                            alloc,
                        ));
                        next_id += 1;
                        budget_used += alloc;
                        youth_requests += 1;
                    }
                }
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 5: Small club squad padding & loan needs
        // ──────────────────────────────────────────────────────────

        let is_small = matches!(
            rep_level,
            ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
        );

        if is_small {
            // Squad too small? Request padding
            if squad.len() < 20 {
                let padding_needed = (20 - squad.len()).min(3);
                for _ in 0..padding_needed {
                    if budget_used >= available_budget {
                        break;
                    }
                    let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
                    if alloc <= 0.0 {
                        break;
                    }

                    // Request generic squad padding
                    requests.push(TransferRequest::new(
                        next_id,
                        PlayerPositionType::MidfielderCenter, // Generic
                        TransferNeedPriority::Optional,
                        TransferNeedReason::SquadPadding,
                        avg_ability.saturating_sub(20),
                        avg_ability.saturating_sub(10),
                        alloc,
                    ));
                    next_id += 1;
                    budget_used += alloc;
                }
            }

            // No experienced players? Request one
            let has_experienced = squad.iter().any(|p| p.age >= 28 && p.current_ability >= avg_ability);
            if !has_experienced && budget_used < available_budget {
                let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
                if alloc > 0.0 {
                    requests.push(TransferRequest::new(
                        next_id,
                        PlayerPositionType::MidfielderCenter,
                        TransferNeedPriority::Optional,
                        TransferNeedReason::ExperiencedHead,
                        avg_ability.saturating_sub(5),
                        avg_ability,
                        alloc,
                    ));
                    next_id += 1;
                    budget_used += alloc;
                }
            }

            // Long-term injuries? Request cover
            let long_injured: Vec<_> = squad
                .iter()
                .filter(|p| p.is_injured && p.recovery_days > 30)
                .collect();
            for injured in long_injured.iter().take(2) {
                if budget_used >= available_budget {
                    break;
                }
                let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
                if alloc <= 0.0 {
                    break;
                }

                requests.push(TransferRequest::new(
                    next_id,
                    injured.primary_position,
                    TransferNeedPriority::Important,
                    TransferNeedReason::InjuryCoverLoan,
                    avg_ability.saturating_sub(15),
                    avg_ability.saturating_sub(5),
                    alloc,
                ));
                next_id += 1;
                budget_used += alloc;
            }
        }

        // ──────────────────────────────────────────────────────────
        // STEP 6: Identify loan-out candidates
        // ──────────────────────────────────────────────────────────

        Self::identify_loan_outs(
            &squad,
            &rep_level,
            avg_ability,
            date,
            players,
            &mut loan_outs,
            &club.philosophy,
            formation_positions,
            current_window,
        );

        SquadEvaluation {
            club_id: club.id,
            requests,
            loan_outs,
            total_budget: budget,
            max_concurrent,
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

    /// Identify loan-out candidates based on club reputation tier.
    fn identify_loan_outs(
        squad: &[SquadPlayerInfo],
        rep_level: &ReputationLevel,
        avg_ability: u8,
        date: NaiveDate,
        players: &[Player],
        loan_outs: &mut Vec<LoanOutCandidate>,
        philosophy: &ClubPhilosophy,
        formation_positions: &[PlayerPositionType; 11],
        current_window: Option<(NaiveDate, NaiveDate)>,
    ) {
        let is_january = Self::is_january_window(date);

        // Philosophy-based loan-out aggressiveness
        let (age_threshold, ability_gap, min_appearances_pct) = match philosophy {
            ClubPhilosophy::DevelopAndSell => (21, 5i16, 30u16),  // Aggressively loan young players
            ClubPhilosophy::SignToCompete => (19, 10i16, 20u16),  // Only loan clearly surplus
            ClubPhilosophy::LoanFocused => (23, 3i16, 40u16),    // Loan to reduce wages
            ClubPhilosophy::Balanced => (21, 8i16, 25u16),       // Standard
        };

        for player_info in squad {
            let player = match players.iter().find(|p| p.id == player_info.player_id) {
                Some(p) => p,
                None => continue,
            };

            // Skip players already on loan
            if player.is_on_loan() {
                continue;
            }

            // Players aged 30+ should not be loaned — they should be sold or released.
            // Loaning older players is unrealistic in real football.
            if player_info.age >= 30 {
                continue;
            }

            // Players loaned out 2+ times should be sold, not loaned again.
            // Repeated loans from the same parent club are unrealistic.
            let previous_loan_count = player.statistics_history.items.iter()
                .filter(|h| h.is_loan)
                .count();
            if previous_loan_count >= 2 {
                continue;
            }

            // Players who are regular contributors (15+ appearances) should not
            // be loaned out — they're getting enough game time already.
            if player_info.appearances >= 15 {
                continue;
            }

            // Same-window protection: signed during this open window → can't be loaned out
            if let (Some(transfer_date), Some((window_start, window_end))) =
                (player.last_transfer_date, current_window)
            {
                if transfer_date >= window_start && transfer_date <= window_end {
                    continue;
                }
            }

            // Club has a signing plan for this player — don't loan them out
            // until they've been properly evaluated (enough time + appearances).
            // Development plans are the exception: loaning IS the plan.
            if let Some(ref plan) = player.plan {
                let total_apps = player_info.appearances;
                if !plan.is_evaluated(date, total_apps) && !plan.is_expired(date)
                    && plan.role != PlayerPlanRole::Development
                {
                    continue;
                }
            }

            let group = player_info.primary_position.position_group();

            // Count players in same position group
            let group_count = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .count();

            // Minimum players needed per group from formation
            let min_needed = match group {
                PlayerFieldPositionGroup::Goalkeeper => 2,
                PlayerFieldPositionGroup::Defender => {
                    formation_positions.iter().filter(|p| p.is_defender()).count() + 1
                }
                PlayerFieldPositionGroup::Midfielder => {
                    formation_positions.iter().filter(|p| p.is_midfielder()).count() + 1
                }
                PlayerFieldPositionGroup::Forward => {
                    formation_positions.iter().filter(|p| p.is_forward()).count()
                }
            };

            // Don't loan out if we'd drop below minimum
            if group_count <= min_needed {
                continue;
            }

            match rep_level {
                ReputationLevel::Elite | ReputationLevel::Continental => {
                    // Young players who need game time
                    if player_info.age <= age_threshold
                        && player_info.potential_ability > player_info.current_ability + 5
                        && (player_info.current_ability as i16) < avg_ability as i16 - ability_gap
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                        });
                        continue;
                    }

                    // Players blocked by better players
                    if player_info.age <= 25
                        && player_info.current_ability >= avg_ability.saturating_sub(10)
                        && player_info.appearances < min_appearances_pct
                    {
                        // Check if there's a clearly better player in same position
                        let better_exists = squad.iter().any(|other| {
                            other.player_id != player_info.player_id
                                && other.primary_position.position_group() == group
                                && other.current_ability > player_info.current_ability + 10
                        });

                        if better_exists {
                            loan_outs.push(LoanOutCandidate {
                                player_id: player_info.player_id,
                                reason: LoanOutReason::BlockedByBetterPlayer,
                                status: LoanOutStatus::Identified,
                                loan_fee: 0.0,
                            });
                            continue;
                        }
                    }

                    // Post-injury fitness
                    if player_info.age <= 25
                        && player_info.is_injured
                        && player_info.recovery_days <= 14
                        && player_info.injury_days > 60
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::PostInjuryFitness,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                        });
                        continue;
                    }

                    // Lack of playing time (January window)
                    if is_january
                        && player_info.age <= 26
                        && player_info.appearances < 5
                        && player_info.current_ability >= avg_ability.saturating_sub(15)
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::LackOfPlayingTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                        });
                        continue;
                    }
                }
                ReputationLevel::National => {
                    // Young players with high potential gap
                    if player_info.age <= 22
                        && player_info.potential_ability > player_info.current_ability + 10
                        && (player_info.current_ability as i16) < avg_ability as i16 - 5
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                        });
                        continue;
                    }

                    // Lack of playing time (January)
                    if is_january && player_info.age <= 24 && player_info.appearances < 3 {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::LackOfPlayingTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                        });
                        continue;
                    }
                }
                _ => {
                    // Regional/Local/Amateur: only loan very young players
                    if player_info.age <= 21
                        && player_info.potential_ability > player_info.current_ability + 15
                        && (player_info.current_ability as i16) < avg_ability as i16 - 10
                    {
                        loan_outs.push(LoanOutCandidate {
                            player_id: player_info.player_id,
                            reason: LoanOutReason::NeedsGameTime,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                        });
                        continue;
                    }
                }
            }

            // Surplus detection (all tiers)
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

            if group_count >= surplus_threshold
                && (player_info.current_ability as i16) < avg_ability as i16 - 5
            {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::Surplus,
                    status: LoanOutStatus::Identified,
                    loan_fee: 0.0,
                });
                continue;
            }

            // Financial relief (LoanFocused philosophy)
            if *philosophy == ClubPhilosophy::LoanFocused
                && (player_info.current_ability as i16) < avg_ability as i16
                && player_info.appearances < 10
            {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::FinancialRelief,
                    status: LoanOutStatus::Identified,
                    loan_fee: 0.0,
                });
            }
        }
    }
}
