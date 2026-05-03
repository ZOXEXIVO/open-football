use chrono::NaiveDate;
use log::debug;
use std::collections::HashMap;

use crate::transfers::TransferWindowManager;
use crate::transfers::pipeline::processor::{PipelineProcessor, SquadPlayerInfo};
use crate::transfers::pipeline::{
    ClubTransferPlan, LoanOutCandidate, LoanOutReason, LoanOutStatus, TransferNeedPriority,
    TransferNeedReason, TransferRequest, TransferRequestStatus,
};
use crate::{
    Club, ClubPhilosophy, Country, MatchTacticType, Person, Player, PlayerFieldPositionGroup,
    PlayerPlanRole, PlayerPositionType, PlayerStatusType, ReputationLevel, TACTICS_POSITIONS,
    TacticsSelector,
};

struct SquadEvaluation {
    club_id: u32,
    requests: Vec<TransferRequest>,
    loan_outs: Vec<LoanOutCandidate>,
    /// Player ids the deterministic position-glut detector wants
    /// hard-listed for transfer this tick. Applied in pass 2 with
    /// mutable Club access. Bypasses the AI listing path because the
    /// signal here is unambiguous: the squad has too many at this
    /// position and the player is too old to loan.
    force_transfer_list: Vec<u32>,
    total_budget: f64,
    max_concurrent: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::transfers::pipeline) enum NeedKind {
    FormationGap,
    QualityUpgrade,
    DepthCover,
}

#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct GroupNeed {
    pub group: PlayerFieldPositionGroup,
    pub representative_pos: PlayerPositionType,
    pub kind: NeedKind,
}

/// Bench depth a group needs in addition to the formation slots —
/// pure function of the formation, used to detect "too thin to cover
/// rotations and injuries." Centralised so evaluation and tests share
/// one source of truth.
pub(in crate::transfers::pipeline) fn group_depth_requirement(
    formation_positions: &[PlayerPositionType; 11],
    group: PlayerFieldPositionGroup,
) -> usize {
    let formation_count = formation_positions
        .iter()
        .filter(|p| p.position_group() == group)
        .count();
    match group {
        PlayerFieldPositionGroup::Goalkeeper => 2,
        PlayerFieldPositionGroup::Defender => formation_count + 2,
        PlayerFieldPositionGroup::Midfielder => formation_count + 1,
        PlayerFieldPositionGroup::Forward => formation_count + 1,
    }
}

/// Detect the recruitment need for each position group, deduped at the
/// group level. Pure function — no side effects, no AI / network.
/// Priority order per group: FormationGap > QualityUpgrade > DepthCover.
/// Each group emits at most one entry, eliminating the slot-level
/// triple-count that distorted budget allocation in the old layout.
pub(in crate::transfers::pipeline) fn compute_group_needs(
    squad: &[crate::transfers::pipeline::processor::SquadPlayerInfo],
    position_coverage: &[(PlayerPositionType, Option<u32>, u8)],
    formation_positions: &[PlayerPositionType; 11],
    rep_score: f32,
    quality_tolerance: i16,
) -> Vec<GroupNeed> {
    let mut needs: Vec<GroupNeed> = Vec::new();
    let mut visited: Vec<PlayerFieldPositionGroup> = Vec::new();

    for (pos, _, _) in position_coverage {
        let group = pos.position_group();
        if visited.contains(&group) {
            continue;
        }
        visited.push(group);

        let representative_pos = formation_positions
            .iter()
            .copied()
            .find(|p| p.position_group() == group)
            .unwrap_or(*pos);

        // (1) Formation gap — any slot in the group has no covering player
        let group_has_gap = position_coverage
            .iter()
            .any(|(p, pid, _)| p.position_group() == group && pid.is_none());
        if group_has_gap {
            needs.push(GroupNeed {
                group,
                representative_pos,
                kind: NeedKind::FormationGap,
            });
            continue;
        }

        // (2) Quality upgrade — best player at this group below tier baseline
        let baseline = PipelineProcessor::tier_starter_ca_score(rep_score, group);
        let best_in_group = squad
            .iter()
            .filter(|p| p.primary_position.position_group() == group)
            .map(|p| p.current_ability)
            .max()
            .unwrap_or(0);
        if (best_in_group as i16) < baseline as i16 - quality_tolerance {
            needs.push(GroupNeed {
                group,
                representative_pos,
                kind: NeedKind::QualityUpgrade,
            });
            continue;
        }

        // (3) Depth cover — group thinner than formation footprint
        let group_count = squad
            .iter()
            .filter(|p| p.primary_position.position_group() == group)
            .count();
        if group_count < group_depth_requirement(formation_positions, group) {
            needs.push(GroupNeed {
                group,
                representative_pos,
                kind: NeedKind::DepthCover,
            });
        }
    }

    needs
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
                    eval.club_id,
                    eval.requests.len(),
                    eval.loan_outs.len(),
                    eval.total_budget
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
                let new_requests: Vec<_> = eval
                    .requests
                    .into_iter()
                    .filter(|new_req| {
                        !plan.transfer_requests.iter().any(|existing| {
                            existing.position.position_group() == new_req.position.position_group()
                                && existing.status != TransferRequestStatus::Fulfilled
                                && existing.status != TransferRequestStatus::Abandoned
                        })
                    })
                    .collect();
                plan.transfer_requests.extend(new_requests);
                // Deduplicate loan-out candidates — don't re-add players already in the list
                for candidate in eval.loan_outs {
                    if !plan
                        .loan_out_candidates
                        .iter()
                        .any(|existing| existing.player_id == candidate.player_id)
                    {
                        plan.loan_out_candidates.push(candidate);
                    }
                }
                plan.last_evaluation_date = Some(date);
                plan.initialized = true;

                // Apply position-glut transfer-list decisions. These
                // are deterministic (not AI-driven) so they bypass
                // `TransferListManager` and write Lst directly. Cheap
                // — usually empty; non-empty only when a club has
                // accumulated a positional surplus the AI hasn't
                // picked off yet (e.g. the Gzira 10-GK case).
                if !eval.force_transfer_list.is_empty() {
                    if let Some(main_team) = club.teams.main_mut() {
                        for player_id in eval.force_transfer_list {
                            if let Some(player) = main_team.players.find_mut(player_id) {
                                if !player.statuses.get().contains(&PlayerStatusType::Lst) {
                                    player.statuses.add(date, PlayerStatusType::Lst);
                                }
                            }
                        }
                    }
                }
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
    fn evaluate_single_club(
        club: &Club,
        date: NaiveDate,
        current_window: Option<(NaiveDate, NaiveDate)>,
    ) -> SquadEvaluation {
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
        let ffp_breach = club.finance.is_ffp_breach(date);
        let budget = if ffp_breach {
            raw_budget * 0.5
        } else {
            raw_budget
        };

        if club.teams.teams.is_empty() {
            return SquadEvaluation {
                club_id: club.id,
                requests,
                loan_outs,
                force_transfer_list: Vec::new(),
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
                force_transfer_list: Vec::new(),
                total_budget: budget,
                max_concurrent: 1,
            };
        }

        // Determine club reputation tier - this drives the entire transfer strategy
        let rep_level = team.reputation.level();

        // Determine max concurrent negotiations by reputation
        let base_max_concurrent = match rep_level {
            ReputationLevel::Elite => 6,
            ReputationLevel::Continental => 5,
            ReputationLevel::National => 3,
            ReputationLevel::Regional => 2,
            _ => 2,
        };
        // FFP breach forces discipline — cap at 1 open negotiation. A real
        // "transfer ban" in everything but name: budget already halved above,
        // and now the club can't spread what it has across multiple targets.
        let max_concurrent = if ffp_breach { 1 } else { base_max_concurrent };

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
            ClubPhilosophy::DevelopAndSell => 25, // accept CA 25 below avg
            ClubPhilosophy::SignToCompete => 5,   // only near or above avg
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

        // Continuous reputation score — drives tier baselines without
        // snapping to enum boundaries. A team mid-Continental gets a
        // different threshold from a team top-of-Continental.
        let rep_score = team.reputation.overall_score();
        let quality_tolerance = Self::tier_quality_tolerance_score(rep_score);
        let _ = ability_tolerance; // philosophy-driven tolerance retained
        // elsewhere; tier baselines drive the
        // recruitment thresholds now.

        // ── Build group-level needs in one pass ──────────────────────
        //
        // Each position group can produce AT MOST one need per evaluation,
        // chosen by priority FormationGap > QualityUpgrade > DepthCover.
        // The previous slot-level construction triple-counted groups in
        // `total_needs`, distorting `budget_per_need`: a back-three with
        // two empty slots looked like "two gaps" for budget purposes
        // even though both were filled by one signing in practice.
        //
        // Detection lives in `compute_group_needs` (pure function, unit
        // tested in helpers' test module).
        let group_needs = compute_group_needs(
            &squad,
            &position_coverage,
            formation_positions,
            rep_score,
            quality_tolerance,
        );

        let total_needs = group_needs.len();
        let budget_per_need = if total_needs > 0 {
            available_budget / total_needs as f64
        } else {
            0.0
        };

        // Generate one request per group with priority and budget
        // appropriate to the kind of need.
        for need in &group_needs {
            let group = need.group;
            let baseline = Self::tier_starter_ca_score(rep_score, group);
            let (priority, reason, mult, min_ca, ideal_ca) = match need.kind {
                NeedKind::FormationGap => (
                    TransferNeedPriority::Critical,
                    TransferNeedReason::FormationGap,
                    1.5,
                    baseline.saturating_sub(12),
                    baseline,
                ),
                NeedKind::QualityUpgrade => (
                    TransferNeedPriority::Important,
                    TransferNeedReason::QualityUpgrade,
                    1.0,
                    baseline.saturating_sub(8),
                    baseline.saturating_add(5),
                ),
                NeedKind::DepthCover => (
                    TransferNeedPriority::Optional,
                    TransferNeedReason::DepthCover,
                    0.6,
                    baseline.saturating_sub(15),
                    baseline.saturating_sub(5),
                ),
            };

            let alloc = (budget_per_need * mult).min(available_budget - budget_used);
            if alloc <= 0.0 {
                break;
            }

            requests.push(TransferRequest::new(
                next_id,
                need.representative_pos,
                priority,
                reason,
                min_ca,
                ideal_ca,
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
        let does_succession = matches!(
            philosophy,
            ClubPhilosophy::SignToCompete | ClubPhilosophy::DevelopAndSell
        ) || matches!(
            rep_level,
            ReputationLevel::Elite | ReputationLevel::Continental
        );
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
                    if !requests
                        .iter()
                        .any(|r| r.position == player_info.primary_position)
                    {
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
            ClubPhilosophy::Balanced => matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental | ReputationLevel::National
            ),
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
                    .filter(|p| {
                        p.primary_position.position_group() == *group && p.age <= youth_age_max
                    })
                    .count();

                // DevelopAndSell wants more youth pipeline depth
                let min_young = if matches!(philosophy, ClubPhilosophy::DevelopAndSell) {
                    3
                } else {
                    2
                };
                if young_in_group < min_young {
                    let alloc = (budget_per_need * 0.3).min(available_budget - budget_used);
                    if alloc <= 0.0 {
                        break;
                    }

                    let pos = match group {
                        PlayerFieldPositionGroup::Defender => PlayerPositionType::DefenderCenter,
                        PlayerFieldPositionGroup::Midfielder => {
                            PlayerPositionType::MidfielderCenter
                        }
                        PlayerFieldPositionGroup::Forward => PlayerPositionType::Striker,
                        _ => continue,
                    };

                    // Don't duplicate if we already have a request for this position group
                    if !requests
                        .iter()
                        .any(|r| r.position.position_group() == *group)
                    {
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
            let has_experienced = squad
                .iter()
                .any(|p| p.age >= 28 && p.current_ability >= avg_ability);
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

        // Position-glut sweep: catches surplus the loan-out branches
        // miss — most importantly the 30+ veterans the loan path
        // explicitly excludes. A club with 8 GKs needs to *eject* the
        // worst, not wait for a deficit signal that never fires when
        // the surplus itself is dragging the average down.
        let force_transfer_list =
            Self::identify_position_glut(&squad, date, players, &mut loan_outs);

        SquadEvaluation {
            club_id: club.id,
            requests,
            loan_outs,
            force_transfer_list,
            total_budget: budget,
            max_concurrent,
        }
    }

    /// Position-glut detector. Independent of age, deficit, or
    /// philosophy: triggers purely on having too many at one position.
    ///
    /// Per group, picks the bottom `count - keep_threshold` players
    /// (worst CA first, oldest tiebreak). Routes them by age:
    ///   * age >= 30 → **transfer-list** (returned for pass-2 apply,
    ///     since the loan path won't take them).
    ///   * age <  30 → **loan-out candidate** with `Surplus` reason.
    ///
    /// Catches the Gzira pattern: 10 GKs sitting on the main roster
    /// because the loan-out branches can't see them (deficit_vs_group
    /// underflows when surplus dominates the average), and the
    /// transfer-list AI hasn't made up its mind. After this fires,
    /// the worst 4-5 GKs are tagged for departure within one tick.
    fn identify_position_glut(
        squad: &[SquadPlayerInfo],
        date: NaiveDate,
        players: &[Player],
        loan_outs: &mut Vec<LoanOutCandidate>,
    ) -> Vec<u32> {
        // Per-position keep ceiling. Anything beyond this is glut.
        // Conservative — leaves headroom for tactical depth (e.g. 5
        // CBs is fine for a back-3 club; 6 starts to be silly).
        const KEEP_GK: usize = 4;
        const KEEP_DEF: usize = 10;
        const KEEP_MID: usize = 10;
        const KEEP_FWD: usize = 7;

        let keep_for = |group: PlayerFieldPositionGroup| -> usize {
            match group {
                PlayerFieldPositionGroup::Goalkeeper => KEEP_GK,
                PlayerFieldPositionGroup::Defender => KEEP_DEF,
                PlayerFieldPositionGroup::Midfielder => KEEP_MID,
                PlayerFieldPositionGroup::Forward => KEEP_FWD,
            }
        };

        let mut force_list: Vec<u32> = Vec::new();
        let groups = [
            PlayerFieldPositionGroup::Goalkeeper,
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward,
        ];

        for group in groups {
            // Rank players in this group by CA ascending (worst first),
            // age descending tiebreak (older first — they're the
            // priority to clear because they can't be loaned).
            let mut ranked: Vec<&SquadPlayerInfo> = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .collect();
            ranked.sort_by(|a, b| {
                a.current_ability
                    .cmp(&b.current_ability)
                    .then(b.age.cmp(&a.age))
            });

            let count = ranked.len();
            let keep = keep_for(group);
            if count <= keep {
                continue;
            }
            let excess = count - keep;

            for surplus in ranked.into_iter().take(excess) {
                // Skip players who are already on loan or already
                // marked for transfer — no need to double-tag, and
                // re-listing churn would invalidate prior negotiations.
                let Some(player) = players.iter().find(|p| p.id == surplus.player_id) else {
                    continue;
                };
                if player.is_on_loan() {
                    continue;
                }
                // Manager-pinned players bypass the position-glut path:
                // even at a positional surplus they don't get listed or
                // loaned out — the pin is the whole point. The pin only
                // applies while the player is under contract; a free
                // agent (contract=None) is a leftover pin from the prior
                // contract and must not block the transfer pipeline.
                if player.is_force_match_selection && player.contract.is_some() {
                    continue;
                }
                let statuses = player.statuses.get();
                let already_listed = statuses.contains(&PlayerStatusType::Lst);

                if surplus.age >= 30 {
                    // Older surplus → transfer-list path (loan path
                    // explicitly excludes >= 30). Skip if already on
                    // the list to avoid stutter.
                    if !already_listed {
                        debug!(
                            "Position glut: forcing Lst on player {} (age {}, CA {}, group {:?})",
                            surplus.player_id, surplus.age, surplus.current_ability, group
                        );
                        force_list.push(surplus.player_id);
                    }
                } else {
                    // Younger surplus → loan-out path. Use the
                    // standard `Surplus` reason so the existing
                    // listing pipeline picks it up.
                    let already_listed_for_loan =
                        loan_outs.iter().any(|c| c.player_id == surplus.player_id);
                    if !already_listed_for_loan {
                        debug!(
                            "Position glut: adding loan-out candidate {} (age {}, CA {}, group {:?})",
                            surplus.player_id, surplus.age, surplus.current_ability, group
                        );
                        loan_outs.push(LoanOutCandidate {
                            player_id: surplus.player_id,
                            reason: LoanOutReason::Surplus,
                            status: LoanOutStatus::Identified,
                            loan_fee: 0.0,
                        });
                    }
                }
            }
        }

        let _ = date; // reserved for future age-window tweaks
        force_list
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
            ClubPhilosophy::DevelopAndSell => (21, 5i16, 30u16), // Aggressively loan young players
            ClubPhilosophy::SignToCompete => (19, 10i16, 20u16), // Only loan clearly surplus
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

            // Manager-pinned: never propose a loan-out, regardless of
            // philosophy / playing-time / surplus signals. The pin is
            // the manager's decision; the AI must respect it. A free
            // agent (no contract) cannot be loaned anyway, but the pin
            // must not block any future move either.
            if player.is_force_match_selection && player.contract.is_some() {
                continue;
            }

            // Players aged 30+ should not be loaned — they should be sold or released.
            // Loaning older players is unrealistic in real football.
            if player_info.age >= 30 {
                continue;
            }

            // Players loaned out 2+ times should be sold, not loaned again.
            // Repeated loans from the same parent club are unrealistic.
            let previous_loan_count = player
                .statistics_history
                .items
                .iter()
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
                if !plan.is_evaluated(date, total_apps)
                    && !plan.is_expired(date)
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
                    formation_positions
                        .iter()
                        .filter(|p| p.is_defender())
                        .count()
                        + 1
                }
                PlayerFieldPositionGroup::Midfielder => {
                    formation_positions
                        .iter()
                        .filter(|p| p.is_midfielder())
                        .count()
                        + 1
                }
                PlayerFieldPositionGroup::Forward => formation_positions
                    .iter()
                    .filter(|p| p.is_forward())
                    .count(),
            };

            // Don't loan out if we'd drop below minimum
            if group_count <= min_needed {
                continue;
            }

            // Depth-chart position in the player's group. Used later as
            // a graduated resistance — the higher up the pecking order
            // a player sits, the harder it is for any loan-out trigger
            // to fire. No hard cut-off: an utterly surplus #1 can still
            // go, it just needs much stronger signals than the 4th-choice
            // would to get there.
            let mut group_ranks: Vec<(u32, u8)> = squad
                .iter()
                .filter(|p| p.primary_position.position_group() == group)
                .map(|p| (p.player_id, p.current_ability))
                .collect();
            group_ranks.sort_by(|a, b| b.1.cmp(&a.1));
            let rank = group_ranks
                .iter()
                .position(|(pid, _)| *pid == player_info.player_id)
                .unwrap_or(usize::MAX);
            // Position-group average CA — compares the player to their own
            // role peer group rather than the outfield-dominated starting
            // XI mean (which quietly branded first-choice keepers "below
            // average" and kept shipping them out).
            let group_avg: u8 = if !group_ranks.is_empty() {
                let sum: u32 = group_ranks.iter().map(|(_, ca)| *ca as u32).sum();
                (sum / group_ranks.len() as u32) as u8
            } else {
                avg_ability
            };
            // Depth cushion: extra CA-below-group-average the player needs
            // to exceed before any "surplus / lack of minutes" branch will
            // fire. Rank 0 (main) needs a massive deficit; rank 3+ needs
            // the normal amount. Scales smoothly; no hard cliff.
            let depth_cushion: i16 = match rank {
                0 => 25,
                1 => 12,
                2 => 5,
                _ => 0,
            };

            match rep_level {
                ReputationLevel::Elite | ReputationLevel::Continental => {
                    // Young players who need game time. Compare to the
                    // position-group average + depth cushion so the main
                    // at any position isn't routed to "dev minutes".
                    if player_info.age <= age_threshold
                        && player_info.potential_ability > player_info.current_ability + 5
                        && (player_info.current_ability as i16)
                            < group_avg as i16 - ability_gap - depth_cushion
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
                    // Young players with high potential gap — group-relative
                    // deficit + depth cushion keeps the starter unscathed.
                    if player_info.age <= 22
                        && player_info.potential_ability > player_info.current_ability + 10
                        && (player_info.current_ability as i16)
                            < group_avg as i16 - 5 - depth_cushion
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
                    // Regional/Local/Amateur: only loan very young players.
                    // Group-relative + depth cushion, same logic as above.
                    if player_info.age <= 21
                        && player_info.potential_ability > player_info.current_ability + 15
                        && (player_info.current_ability as i16)
                            < group_avg as i16 - 10 - depth_cushion
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

            // Surplus fires on a position-group-relative deficit — a GK
            // sitting below the outfield-dominated squad mean is normal.
            // Depth cushion makes the first-choice extremely hard to flag.
            let deficit_vs_group = group_avg as i16 - player_info.current_ability as i16;
            if group_count >= surplus_threshold && deficit_vs_group >= 5 + depth_cushion {
                loan_outs.push(LoanOutCandidate {
                    player_id: player_info.player_id,
                    reason: LoanOutReason::Surplus,
                    status: LoanOutStatus::Identified,
                    loan_fee: 0.0,
                });
                continue;
            }

            // Financial relief (LoanFocused philosophy). The depth cushion
            // protects starters here too — you don't dump your first-choice
            // for wage relief.
            if *philosophy == ClubPhilosophy::LoanFocused
                && deficit_vs_group >= depth_cushion
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
