mod composition;
mod demotion;
mod promotion;
mod recall;
mod satisfaction;
mod swap;
mod transfer_listing;

pub(crate) mod legacy;

pub use composition::SquadComposition;
pub use demotion::DemotionEvaluator;
pub use promotion::YouthPromotionEvaluator;
pub use recall::RecallEvaluator;
pub use satisfaction::compute_squad_satisfaction;
pub use swap::AbilitySwapEvaluator;
pub use transfer_listing::TransferListManager;

use crate::club::team::coach_perception::{CoachDecisionState, RecentMoveType};
use crate::utils::DateUtils;
use crate::Team;
use chrono::NaiveDate;

pub struct SquadManager;

impl SquadManager {
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
        let coach_name = teams[main_idx].staffs.head_coach().full_name.to_string();
        let demotions = DemotionEvaluator::identify_administrative_demotions(&teams[main_idx]);
        let max_age = teams[reserve_idx].team_type.max_age();
        let demotions = filter_by_age(demotions, &teams[main_idx], max_age, date);
        if !demotions.is_empty() {
            execute_moves(teams, main_idx, reserve_idx, &demotions);
            record_player_decisions(teams, main_idx, reserve_idx, &demotions, date, &coach_name, "Administrative demotion");
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

pub(crate) fn execute_moves(teams: &mut [Team], from_idx: usize, to_idx: usize, player_ids: &[u32]) {
    for &player_id in player_ids {
        if let Some(player) = teams[from_idx].players.take_player(&player_id) {
            teams[from_idx].transfer_list.remove(player_id);
            teams[to_idx].players.add(player);
        }
    }
}

fn team_label(team: &Team) -> String {
    match team.team_type {
        crate::TeamType::Main => team.name.clone(),
        _ => format!("{} {}", team.name, team.team_type.as_i18n_key().replace('_', " ")),
    }
}

pub(crate) fn record_player_decisions(
    teams: &mut [Team],
    from_idx: usize,
    to_idx: usize,
    player_ids: &[u32],
    date: NaiveDate,
    decided_by: &str,
    reason: &str,
) {
    let from_label = team_label(&teams[from_idx]);
    let to_label = team_label(&teams[to_idx]);
    let movement = format!("{} → {}", from_label, to_label);
    for &pid in player_ids {
        if let Some(player) = teams[to_idx].players.players.iter_mut().find(|p| p.id == pid) {
            player.decision_history.add(date, movement.clone(), reason.to_string(), decided_by.to_string());
        }
    }
}

pub(crate) fn filter_by_age(
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

pub(crate) fn record_moves(
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
