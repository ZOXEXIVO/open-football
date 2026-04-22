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

    // STEP 1: Goalkeeper. Fallback order:
    //   1. Best available keeper (fit, not injured, not on int duty).
    //   2. Any keeper in the available pool, even if low-condition — we
    //      normally reject those, but a tired keeper still has real
    //      goalkeeping skills and saves far more than an outfielder
    //      pressed into goal. Skipped only if the keeper is actively
    //      injured or banned (those shouldn't play at all).
    //   3. Last resort: outfielder as emergency keeper. Real football
    //      does this but the result is a 5+ goal concession — so we
    //      reach this only when the club has literally no keeper on
    //      the roster.
    //
    // The Goalkeeping struct on an outfielder defaults to all zeros
    // (never trained as a keeper), which previously produced hnd=1
    // ref=1 after the (x-1)/19 scaling clamp — save rate effectively
    // 0%, and the league generated repeatable 10+ goal blowouts. This
    // order keeps a real keeper in goal whenever possible.
    let picked_gk = pick_best_goalkeeper(available, &used_ids, engine, staff, is_friendly, match_importance)
        .or_else(|| pick_any_goalkeeper_fallback(available, &used_ids));
    if let Some(gk) = picked_gk {
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
            .filter(|p| !p.positions.is_goalkeeper())
            .max_by(|a, b| {
                let sa = engine.score_player_for_slot(a, pos, target_group, staff, tactics, date, is_friendly, &selected_players)
                    + engine.development_minutes_bonus(a, match_importance)
                    + engine.fatigue_penalty(a, is_friendly);
                let sb = engine.score_player_for_slot(b, pos, target_group, staff, tactics, date, is_friendly, &selected_players)
                    + engine.development_minutes_bonus(b, match_importance)
                    + engine.fatigue_penalty(b, is_friendly);
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
            .filter(|p| !p.positions.is_goalkeeper())
            .max_by(|a, b| {
                let sa = engine.overall_quality(a, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(a, match_importance)
                    + engine.fatigue_penalty(a, is_friendly);
                let sb = engine.overall_quality(b, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(b, match_importance)
                    + engine.fatigue_penalty(b, is_friendly);
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
    if let Some(gk) = pick_best_goalkeeper(remaining, &used_ids, engine, staff, is_friendly, match_importance) {
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
                    + engine.development_minutes_bonus(a, match_importance)
                    + engine.fatigue_penalty(a, is_friendly);
                let sb = engine.overall_quality(b, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(b, match_importance)
                    + engine.fatigue_penalty(b, is_friendly);
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
                    + engine.development_minutes_bonus(a, match_importance)
                    + engine.fatigue_penalty(a, is_friendly);
                let sb = engine.overall_quality(b, staff, tactics, date, is_friendly)
                    + engine.development_minutes_bonus(b, match_importance)
                    + engine.fatigue_penalty(b, is_friendly);
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

/// Skill-blind keeper fallback. `pick_best_goalkeeper` uses a full
/// scoring pipeline (ability × age × form × staff opinion) which can
/// theoretically return None if every scored keeper produces NaN or
/// similar edge values. This variant just picks any player in the
/// available pool whose registered positions include Goalkeeper —
/// preferring the one with highest combined handling+reflexes so the
/// walking-wounded keeper with a real goalkeeping profile is picked
/// over the fresh outfielder with a zeroed one. Used as the second
/// line of the keeper fallback chain before the outfielder-as-GK
/// emergency path.
fn pick_any_goalkeeper_fallback<'p>(
    available: &[&'p Player],
    used_ids: &[u32],
) -> Option<&'p Player> {
    available
        .iter()
        .filter(|p| !used_ids.contains(&p.id))
        .filter(|p| p.positions.is_goalkeeper())
        .max_by(|a, b| {
            let sa = a.skills.goalkeeping.handling + a.skills.goalkeeping.reflexes;
            let sb = b.skills.goalkeeping.handling + b.skills.goalkeeping.reflexes;
            sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}

fn pick_best_goalkeeper<'p>(
    available: &[&'p Player],
    used_ids: &[u32],
    engine: &ScoringEngine,
    staff: &Staff,
    is_friendly: bool,
    match_importance: f32,
) -> Option<&'p Player> {
    // In real football the #1 keeper plays everything unless injured,
    // genuinely out of form, or the fixture is low priority (early cup
    // rounds, dead rubbers). Injury/suspension is already filtered before
    // we get here; poor form is baked into `goalkeeper_score` via
    // match_readiness + condition_floor_penalty. The missing rotation
    // trigger was fixture importance — `development_minutes_bonus` only
    // fires when match_importance < 0.5, giving an underplayed backup a
    // boost on cup nights but vanishing for league games, so the #1 GK
    // isn't displaced by a workload signal that doesn't apply to keepers.
    available
        .iter()
        .filter(|p| !used_ids.contains(&p.id))
        .filter(|p| p.positions.is_goalkeeper())
        .max_by(|a, b| {
            let score_a = engine.goalkeeper_score(a, staff, is_friendly)
                + engine.development_minutes_bonus(a, match_importance);
            let score_b = engine.goalkeeper_score(b, staff, is_friendly)
                + engine.development_minutes_bonus(b, match_importance);
            score_a.partial_cmp(&score_b).unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}
