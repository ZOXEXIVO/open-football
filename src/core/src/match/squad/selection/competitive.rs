use crate::club::{PlayerFieldPositionGroup, PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::{Player, Tactics};
use chrono::NaiveDate;
use log::debug;

use super::helpers;
use super::scoring::ScoringEngine;

/// Select the best starting 11 for competitive matches.
pub(crate) fn select_starting_eleven(
    team_id: u32,
    available: &[&Player],
    staff: &Staff,
    tactics: &Tactics,
    engine: &ScoringEngine,
    date: NaiveDate,
    is_friendly: bool,
    match_importance: f32,
) -> Vec<MatchPlayer> {
    let mut squad: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_SQUAD_SIZE);
    let mut used_ids: Vec<u32> = Vec::new();
    let mut selected_players: Vec<&Player> = Vec::new();
    let required = tactics.positions();

    // STEP 1: Goalkeeper
    if let Some(gk) = pick_best_goalkeeper(available, &used_ids, engine, staff, is_friendly) {
        squad.push(MatchPlayer::from_player(team_id, gk, PlayerPositionType::Goalkeeper, false));
        used_ids.push(gk.id);
        selected_players.push(gk);
    } else {
        debug!("No goalkeeper found at all — picking any player as GK");
        if let Some(any) = helpers::pick_best_unused(available, &used_ids) {
            squad.push(MatchPlayer::from_player(team_id, any, PlayerPositionType::Goalkeeper, false));
            used_ids.push(any.id);
            selected_players.push(any);
        }
    }

    // STEP 2: Fill each outfield position
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
                let sa = engine.score_player_for_slot(a, pos, target_group, staff, tactics, date, is_friendly, &selected_players)
                    + engine.development_minutes_bonus(a, match_importance);
                let sb = engine.score_player_for_slot(b, pos, target_group, staff, tactics, date, is_friendly, &selected_players)
                    + engine.development_minutes_bonus(b, match_importance);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied();

        if let Some(player) = best {
            squad.push(MatchPlayer::from_player(team_id, player, pos, false));
            used_ids.push(player.id);
            selected_players.push(player);
        }
    }

    // STEP 3: Fill remaining slots with best available
    while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
        let best = available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| !helpers::is_goalkeeper_player(p))
            .max_by(|a, b| {
                let sa = engine.overall_quality(a, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(a, match_importance);
                let sb = engine.overall_quality(b, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(b, match_importance);
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

    // STEP 4: LAST RESORT — use ANY remaining player
    while squad.len() < helpers::DEFAULT_SQUAD_SIZE {
        let best = available
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = a.player_attributes.current_ability;
                let sb = b.player_attributes.current_ability;
                sa.cmp(&sb)
            })
            .copied();

        match best {
            Some(player) => {
                let pos = helpers::best_tactical_position(player, tactics);
                debug!("Emergency fill: using {} as outfield player", player.full_name);
                squad.push(MatchPlayer::from_player(team_id, player, pos, false));
                used_ids.push(player.id);
            }
            None => break,
        }
    }

    if squad.len() < helpers::DEFAULT_SQUAD_SIZE {
        debug!("Could only select {} of 11 starting players", squad.len());
    }

    squad
}

/// Select substitutes for competitive matches.
pub(crate) fn select_substitutes(
    team_id: u32,
    remaining: &[&Player],
    staff: &Staff,
    tactics: &Tactics,
    engine: &ScoringEngine,
    date: NaiveDate,
    is_friendly: bool,
    match_importance: f32,
) -> Vec<MatchPlayer> {
    let mut subs: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_BENCH_SIZE);
    let mut used_ids: Vec<u32> = Vec::new();

    // 1. Backup goalkeeper
    if let Some(gk) = pick_best_goalkeeper(remaining, &used_ids, engine, staff, is_friendly) {
        subs.push(MatchPlayer::from_player(team_id, gk, PlayerPositionType::Goalkeeper, false));
        used_ids.push(gk.id);
    }

    // 2. Positional variety
    for target_group in &[
        PlayerFieldPositionGroup::Defender,
        PlayerFieldPositionGroup::Midfielder,
        PlayerFieldPositionGroup::Forward,
    ] {
        if subs.len() >= helpers::DEFAULT_BENCH_SIZE {
            break;
        }
        let has_group = subs.iter().any(|s| {
            s.tactical_position.current_position.position_group() == *target_group
        });
        if has_group {
            continue;
        }

        let best = remaining
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .filter(|p| p.position().position_group() == *target_group)
            .max_by(|a, b| {
                let sa = engine.overall_quality(a, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(a, match_importance);
                let sb = engine.overall_quality(b, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(b, match_importance);
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied();

        if let Some(player) = best {
            let pos = helpers::best_tactical_position(player, tactics);
            subs.push(MatchPlayer::from_player(team_id, player, pos, false));
            used_ids.push(player.id);
        }
    }

    // 3. Fill remaining with best available
    while subs.len() < helpers::DEFAULT_BENCH_SIZE {
        let best = remaining
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = engine.overall_quality(a, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(a, match_importance);
                let sb = engine.overall_quality(b, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(b, match_importance);
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

fn pick_best_goalkeeper<'p>(
    available: &[&'p Player],
    used_ids: &[u32],
    engine: &ScoringEngine,
    staff: &Staff,
    is_friendly: bool,
) -> Option<&'p Player> {
    available
        .iter()
        .filter(|p| !used_ids.contains(&p.id))
        .filter(|p| helpers::is_goalkeeper_player(p))
        .max_by(|a, b| {
            let score_a = engine.goalkeeper_score(a, staff, is_friendly);
            let score_b = engine.goalkeeper_score(b, staff, is_friendly);
            score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}
