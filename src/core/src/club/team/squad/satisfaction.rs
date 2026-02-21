use crate::club::team::coach_perception::CoachDecisionState;
use crate::{Player, PlayerFieldPositionGroup, Team};

pub fn compute_squad_satisfaction(main_team: &Team, state: &CoachDecisionState) -> f32 {
    let players = &main_team.players.players;
    let squad_size = players.len();

    let size_satisfaction = if (20..=23).contains(&squad_size) { 1.0 }
        else if squad_size >= 18 && squad_size <= 25 { 0.7 }
        else if squad_size >= 14 { 0.4 }
        else { 0.1 };

    let played_players: Vec<&Player> = players.iter()
        .filter(|p| p.statistics.played + p.statistics.played_subs > 3).collect();
    let perf_satisfaction = if played_players.is_empty() { 0.5 } else {
        let avg_rating: f32 = played_players.iter()
            .map(|p| p.statistics.average_rating).sum::<f32>() / played_players.len() as f32;
        ((avg_rating - 5.5) / 2.0).clamp(0.0, 1.0)
    };

    let qualities: Vec<f32> = players.iter()
        .filter_map(|p| state.impressions.get(&p.id).map(|imp| imp.perceived_quality))
        .collect();
    let spread_satisfaction = if qualities.len() >= 2 {
        let max_q = qualities.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        let min_q = qualities.iter().cloned().fold(f32::INFINITY, f32::min);
        (1.0 - (max_q - min_q) / 10.0).clamp(0.0, 1.0)
    } else { 0.5 };

    let available: Vec<_> = players.iter().filter(|p| !p.player_attributes.is_injured).collect();
    let has_gk = available.iter().any(|p| p.position().position_group() == PlayerFieldPositionGroup::Goalkeeper);
    let has_def = available.iter().filter(|p| p.position().position_group() == PlayerFieldPositionGroup::Defender).count() >= 3;
    let has_mid = available.iter().filter(|p| p.position().position_group() == PlayerFieldPositionGroup::Midfielder).count() >= 2;
    let has_fwd = available.iter().filter(|p| p.position().position_group() == PlayerFieldPositionGroup::Forward).count() >= 1;
    let coverage_satisfaction = if has_gk && has_def && has_mid && has_fwd { 1.0 } else { 0.2 };

    size_satisfaction * 0.25 + perf_satisfaction * 0.35
        + spread_satisfaction * 0.15 + coverage_satisfaction * 0.25
}
