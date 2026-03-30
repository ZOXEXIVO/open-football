use crate::club::PlayerPositionType;
use crate::r#match::player::MatchPlayer;
use crate::{Player, Staff, Tactics};

use super::helpers;

/// Select starting 11 with rotation priority (friendly/development matches).
pub(crate) fn select_rotation_starting_eleven(
    team_id: u32,
    available: &[&Player],
    staff: &Staff,
    tactics: &Tactics,
) -> Vec<MatchPlayer> {
    let mut squad: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_SQUAD_SIZE);
    let mut used_ids: Vec<u32> = Vec::new();
    let required = tactics.positions();

    if let Some(gk) = pick_rotation_goalkeeper(available, &used_ids) {
        squad.push(MatchPlayer::from_player(team_id, gk, PlayerPositionType::Goalkeeper, false));
        used_ids.push(gk.id);
    } else if let Some(any) = helpers::pick_best_unused(available, &used_ids) {
        squad.push(MatchPlayer::from_player(team_id, any, PlayerPositionType::Goalkeeper, false));
        used_ids.push(any.id);
    }

    for &pos in required.iter() {
        if pos == PlayerPositionType::Goalkeeper {
            continue;
        }

        let target_group = pos.position_group();

        let best = available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| !helpers::is_goalkeeper_player(p))
            .max_by(|a, b| {
                let sa = rotation_score_for_slot(a, pos, target_group, staff, tactics);
                let sb = rotation_score_for_slot(b, pos, target_group, staff, tactics);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied();

        if let Some(player) = best {
            squad.push(MatchPlayer::from_player(team_id, player, pos, false));
            used_ids.push(player.id);
        }
    }

    while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
        let best = available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| !helpers::is_goalkeeper_player(p))
            .max_by(|a, b| {
                let sa = rotation_overall_quality(a);
                let sb = rotation_overall_quality(b);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied();

        match best {
            Some(player) => {
                let pos = helpers::best_tactical_position(player, tactics);
                squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
            None => break,
        }
    }

    // Last resort — any player
    while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
        let best = available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = a.player_attributes.days_since_last_match;
                let sb = b.player_attributes.days_since_last_match;
                sa.cmp(&sb)
            })
            .copied();

        match best {
            Some(player) => {
                let pos = helpers::best_tactical_position(player, tactics);
                squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
            None => break,
        }
    }

    squad
}

/// Select substitutes with rotation priority.
pub(crate) fn select_rotation_substitutes(
    team_id: u32,
    remaining: &[&Player],
    _staff: &Staff,
    tactics: &Tactics,
) -> Vec<MatchPlayer> {
    let mut subs: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_BENCH_SIZE);
    let mut used_ids: Vec<u32> = Vec::new();

    if let Some(gk) = pick_rotation_goalkeeper(remaining, &used_ids) {
        subs.push(MatchPlayer::from_player(team_id, gk, PlayerPositionType::Goalkeeper, false));
        used_ids.push(gk.id);
    }

    while subs.len() < helpers::DEFAULT_BENCH_SIZE {
        let best = remaining
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = rotation_overall_quality(a);
                let sb = rotation_overall_quality(b);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied();

        match best {
            Some(player) => {
                let pos = helpers::best_tactical_position(player, tactics);
                subs.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
            None => break,
        }
    }

    subs
}

fn rotation_score_for_slot(
    player: &Player,
    slot_position: PlayerPositionType,
    slot_group: crate::club::PlayerFieldPositionGroup,
    _staff: &Staff,
    _tactics: &Tactics,
) -> f32 {
    let condition_pct = player.player_attributes.condition_percentage() as f32;

    if condition_pct < 20.0 {
        let deficit = (20.0 - condition_pct) / 20.0;
        return -(deficit * 30.0);
    }

    let mut score: f32 = 0.0;

    let days = player.player_attributes.days_since_last_match as f32;
    let rest_score = (days / 14.0).min(1.0) * 20.0;
    score += rest_score * 0.35;

    let position_fit = helpers::position_fit_score(player, slot_position, slot_group);
    score += position_fit * 0.30;

    let condition_norm = (condition_pct / 100.0).clamp(0.0, 1.0);
    score += condition_norm * 20.0 * 0.20;

    let ability = player.player_attributes.current_ability as f32 / 200.0;
    score += ability * 20.0 * 0.15;

    score
}

fn rotation_overall_quality(player: &Player) -> f32 {
    let condition_pct = player.player_attributes.condition_percentage() as f32;

    if condition_pct < 20.0 {
        let deficit = (20.0 - condition_pct) / 20.0;
        return -(deficit * 30.0);
    }

    let days = player.player_attributes.days_since_last_match as f32;
    let rest_score = (days / 14.0).min(1.0) * 20.0;
    let condition_norm = (condition_pct / 100.0).clamp(0.0, 1.0) * 20.0;

    rest_score * 0.40
        + condition_norm * 0.35
        + (player.player_attributes.current_ability as f32 / 200.0 * 20.0) * 0.25
}

fn pick_rotation_goalkeeper<'p>(
    available: &[&'p Player],
    used_ids: &[u32],
) -> Option<&'p Player> {
    available
        .iter()
        .filter(|p| !used_ids.contains(&p.id))
        .filter(|p| helpers::is_goalkeeper_player(p))
        .filter(|p| p.player_attributes.condition_percentage() >= 20)
        .max_by(|a, b| {
            let ca = a.player_attributes.condition_percentage();
            let cb = b.player_attributes.condition_percentage();
            ca.cmp(&cb).then_with(|| {
                a.player_attributes.days_since_last_match
                    .cmp(&b.player_attributes.days_since_last_match)
            })
        })
        .copied()
}
