use crate::club::staff::perception::CoachDecisionState;
use crate::{PlayerFieldPositionGroup, Team};

/// Computes overall squad satisfaction (0.0–1.0) from four weighted factors:
///   - Squad size (25%)
///   - Performance (35%)
///   - Quality spread (15%)
///   - Position coverage (25%)
pub fn compute_squad_satisfaction(main_team: &Team, state: &CoachDecisionState) -> f32 {
    let players = &main_team.players.players;

    let size = size_satisfaction(players.len());
    let performance = performance_satisfaction(players);
    let spread = quality_spread_satisfaction(players, state);
    let coverage = position_coverage_satisfaction(players);

    size * 0.25 + performance * 0.35 + spread * 0.15 + coverage * 0.25
}

/// Optimal squad is 20–23 players.
fn size_satisfaction(squad_size: usize) -> f32 {
    match squad_size {
        20..=23 => 1.0,
        18..=25 => 0.7,
        14..=17 => 0.4,
        _ => 0.1,
    }
}

/// Based on average match rating of players with 4+ appearances.
fn performance_satisfaction(players: &[crate::Player]) -> f32 {
    let experienced: Vec<_> = players
        .iter()
        .filter(|p| p.statistics.played + p.statistics.played_subs > 3)
        .collect();

    if experienced.is_empty() {
        return 0.5;
    }

    let avg_rating: f32 =
        experienced.iter().map(|p| p.statistics.average_rating).sum::<f32>()
            / experienced.len() as f32;

    // Map rating 5.5–7.5 onto 0.0–1.0
    ((avg_rating - 5.5) / 2.0).clamp(0.0, 1.0)
}

/// Penalises large gaps between best and worst perceived quality.
fn quality_spread_satisfaction(
    players: &[crate::Player],
    state: &CoachDecisionState,
) -> f32 {
    let qualities: Vec<f32> = players
        .iter()
        .filter_map(|p| state.impressions.get(&p.id).map(|imp| imp.perceived_quality))
        .collect();

    if qualities.len() < 2 {
        return 0.5;
    }

    let max_q = qualities.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let min_q = qualities.iter().cloned().fold(f32::INFINITY, f32::min);

    // Gap of 10+ → 0.0, gap of 0 → 1.0
    (1.0 - (max_q - min_q) / 10.0).clamp(0.0, 1.0)
}

/// Checks minimum position coverage among healthy players:
/// 1 GK, 3 DEF, 2 MID, 1 FWD.
fn position_coverage_satisfaction(players: &[crate::Player]) -> f32 {
    let available: Vec<_> = players
        .iter()
        .filter(|p| !p.player_attributes.is_injured)
        .collect();

    let count = |group: PlayerFieldPositionGroup| -> usize {
        available.iter().filter(|p| p.position().position_group() == group).count()
    };

    let covered = count(PlayerFieldPositionGroup::Goalkeeper) >= 1
        && count(PlayerFieldPositionGroup::Defender) >= 3
        && count(PlayerFieldPositionGroup::Midfielder) >= 2
        && count(PlayerFieldPositionGroup::Forward) >= 1;

    if covered { 1.0 } else { 0.2 }
}
