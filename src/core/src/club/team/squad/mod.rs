mod demotion;
mod promotion;
mod recall;
mod satisfaction;
mod swap;

pub(crate) mod legacy;

pub use demotion::DemotionEvaluator;
pub use promotion::YouthPromotionEvaluator;
pub use recall::RecallEvaluator;
pub use satisfaction::compute_squad_satisfaction;
pub use swap::AbilitySwapEvaluator;

use crate::club::team::coach_perception::{CoachDecisionState, RecentMoveType};
use crate::utils::DateUtils;
use crate::{PlayerFieldPositionGroup, Team};
use chrono::NaiveDate;
use log::debug;

use crate::club::team::coach_perception::seeded_decision;

pub struct SquadManager;

impl SquadManager {
    /// Weekly: full squad review (demotions -> recalls -> youth promotions)
    pub fn manage_composition(
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: Option<usize>,
        youth_idx: Option<usize>,
        date: NaiveDate,
    ) {
        // --- Trigger detection & pressure-based gating ---
        let should_act = if let Some(state) = coach_state {
            let main_team = &teams[main_idx];
            let satisfaction = compute_squad_satisfaction(main_team, state);
            state.squad_satisfaction = satisfaction;
            state.weeks_since_last_change += 1;

            // Decay trigger pressure each cycle
            state.trigger_pressure *= 0.85;

            // Accumulate triggers from observable events
            let injured_count = main_team.players.players.iter()
                .filter(|p| p.player_attributes.is_injured)
                .count();
            if injured_count >= 4 {
                state.trigger_pressure += 0.3;
            }

            let played_players: Vec<_> = main_team.players.players.iter()
                .filter(|p| p.statistics.played + p.statistics.played_subs > 3)
                .collect();
            if !played_players.is_empty() {
                let avg_form: f32 = played_players.iter()
                    .map(|p| p.statistics.average_rating)
                    .sum::<f32>() / played_players.len() as f32;
                if avg_form < 6.0 {
                    state.trigger_pressure += 0.2;
                    state.emotional_heat = (state.emotional_heat + 0.15).clamp(0.0, 1.0);
                }
            }

            // Position emergency: any group with 0 available
            let available: Vec<_> = main_team.players.players.iter()
                .filter(|p| !p.player_attributes.is_injured)
                .collect();
            let has_gk = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Goalkeeper);
            let has_def = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Defender);
            let has_mid = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Midfielder);
            let has_fwd = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Forward);
            if !has_gk || !has_def || !has_mid || !has_fwd {
                state.trigger_pressure += 0.4;
            }

            // Time pressure: not looking at squad for too long builds restlessness
            if state.weeks_since_last_change > 6 {
                state.trigger_pressure += (state.weeks_since_last_change - 6) as f32 * 0.05;
            }
            state.trigger_pressure = state.trigger_pressure.clamp(0.0, 1.0);

            // Compute action drive vs inertia pull
            let action_drive = state.trigger_pressure * (1.0 - state.profile.conservatism * 0.3)
                + (1.0 - satisfaction) * 0.3
                + state.emotional_heat * 0.2;
            let inertia_pull = state.profile.conservatism * 0.3
                + satisfaction * 0.2
                + if state.weeks_since_last_change < 3 { 0.3 } else { 0.0 };

            let action_prob = (action_drive - inertia_pull + 0.15).clamp(0.05, 0.95);
            let squad_size = main_team.players.players.len();

            let seed = state.profile.coach_seed
                .wrapping_mul(state.current_week)
                .wrapping_add(0xFA57);

            // Emergency override: always act if squad too small
            if squad_size < 16 {
                true
            } else if seeded_decision(action_prob, seed) {
                true
            } else {
                debug!(
                    "Squad management: coach not triggered (pressure={:.2}, satisfaction={:.2}, heat={:.2})",
                    state.trigger_pressure, satisfaction, state.emotional_heat
                );
                false
            }
        } else {
            true // no coach state -> use legacy
        };

        if !should_act {
            return;
        }

        let mut any_move = false;

        // Phase 1: Demotions (main -> reserves)
        if let Some(res_idx) = reserve_idx {
            let demotions = DemotionEvaluator::evaluate(teams, main_idx, coach_state.as_ref(), date);
            let max_age = teams[res_idx].team_type.max_age();
            let demotions = filter_by_age(demotions, &teams[main_idx], max_age, date);
            if !demotions.is_empty() {
                debug!(
                    "Squad management: demoting {} players to reserves",
                    demotions.len()
                );
                execute_moves(teams, main_idx, res_idx, &demotions);
                record_moves(coach_state, &demotions, RecentMoveType::DemotedToReserves, date);
                any_move = true;
            }
        }

        // Phase 2: Recalls (reserves -> main)
        if let Some(res_idx) = reserve_idx {
            let recalls = RecallEvaluator::evaluate(teams, main_idx, res_idx, coach_state.as_ref(), date);
            if !recalls.is_empty() {
                debug!(
                    "Squad management: recalling {} players from reserves",
                    recalls.len()
                );
                execute_moves(teams, res_idx, main_idx, &recalls);
                record_moves(coach_state, &recalls, RecentMoveType::RecalledFromReserves, date);
                any_move = true;
            }
        }

        // Phase 3: Youth promotions
        if let Some(y_idx) = youth_idx {
            let promotions = YouthPromotionEvaluator::evaluate(teams, main_idx, y_idx, coach_state.as_ref(), date);
            if !promotions.is_empty() {
                debug!(
                    "Squad management: promoting {} youth players",
                    promotions.len()
                );
                execute_moves(teams, y_idx, main_idx, &promotions);
                record_moves(coach_state, &promotions, RecentMoveType::YouthPromoted, date);
                any_move = true;
            }
        }

        if any_move {
            if let Some(state) = coach_state {
                state.weeks_since_last_change = 0;
            }
        }
    }

    /// Daily: only mandatory administrative demotions (Lst, Loa).
    /// All other squad decisions (recalls, swaps, performance demotions)
    /// go through the trigger-gated weekly manage_composition.
    pub fn manage_critical_moves(
        teams: &mut [Team],
        coach_state: &mut Option<CoachDecisionState>,
        main_idx: usize,
        reserve_idx: usize,
        date: NaiveDate,
    ) {
        let demotions = DemotionEvaluator::identify_administrative_demotions(&teams[main_idx]);
        let max_age = teams[reserve_idx].team_type.max_age();
        let demotions = filter_by_age(demotions, &teams[main_idx], max_age, date);
        if !demotions.is_empty() {
            debug!(
                "Daily squad moves: administrative demotion of {} players (Lst/Loa)",
                demotions.len()
            );
            execute_moves(teams, main_idx, reserve_idx, &demotions);
            record_moves(coach_state, &demotions, RecentMoveType::DemotedToReserves, date);

            // Administrative demotions increase trigger pressure so the weekly
            // review reacts faster (recalls, replacements).
            if let Some(state) = coach_state {
                state.trigger_pressure = (state.trigger_pressure + 0.15 * demotions.len() as f32)
                    .clamp(0.0, 1.0);
            }
        }
    }
}

// ─── Shared helpers ─────────────────────────────────────────────────

pub fn execute_moves(teams: &mut [Team], from_idx: usize, to_idx: usize, player_ids: &[u32]) {
    for &player_id in player_ids {
        if let Some(player) = teams[from_idx].players.take_player(&player_id) {
            teams[from_idx].transfer_list.remove(player_id);
            teams[to_idx].players.add(player);
        }
    }
}

pub fn filter_by_age(
    ids: Vec<u32>,
    team: &Team,
    max_age: Option<u8>,
    date: NaiveDate,
) -> Vec<u32> {
    match max_age {
        Some(max) => ids
            .into_iter()
            .filter(|&pid| {
                team.players
                    .players
                    .iter()
                    .find(|p| p.id == pid)
                    .map(|p| DateUtils::age(p.birth_date, date) <= max)
                    .unwrap_or(false)
            })
            .collect(),
        None => ids,
    }
}

fn record_moves(
    coach_state: &mut Option<CoachDecisionState>,
    ids: &[u32],
    move_type: RecentMoveType,
    date: NaiveDate,
) {
    if let Some(state) = coach_state {
        for &id in ids {
            state.record_move(id, move_type, date);
        }
    }
}
