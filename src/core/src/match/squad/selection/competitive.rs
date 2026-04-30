use crate::club::{PlayerPositionType, Staff};
use crate::r#match::player::MatchPlayer;
use crate::utils::DateUtils;
use crate::{Player, PlayerSquadStatus, Tactics};
use chrono::NaiveDate;
use log::debug;

use super::SelectionPolicy;
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
    policy: SelectionPolicy,
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
    let picked_gk = pick_best_goalkeeper(
        available,
        &used_ids,
        engine,
        staff,
        is_friendly,
        match_importance,
    )
    .or_else(|| pick_any_goalkeeper_fallback(available, &used_ids));
    if let Some(gk) = picked_gk {
        squad.push(MatchPlayer::from_player(
            team_id,
            gk,
            PlayerPositionType::Goalkeeper,
            false,
        ));
        used_ids.push(gk.id);
        selected_players.push(gk);
    } else {
        debug!("No goalkeeper found at all — picking any player as GK");
        if let Some(any) = helpers::pick_best_unused(available, &used_ids) {
            squad.push(MatchPlayer::from_player(
                team_id,
                any,
                PlayerPositionType::Goalkeeper,
                false,
            ));
            used_ids.push(any.id);
            selected_players.push(any);
        }
    }

    // STEP 2: Fill outfield positions as one assignment problem. This avoids
    // burning a versatile player in an early slot when a specialist is needed
    // later in the shape.
    let outfield_slots: Vec<PlayerPositionType> = required
        .iter()
        .copied()
        .filter(|p| *p != PlayerPositionType::Goalkeeper)
        .collect();
    let assignments = assign_outfield_slots(
        available,
        &used_ids,
        &outfield_slots,
        staff,
        tactics,
        engine,
        date,
        is_friendly,
        match_importance,
        policy,
    );

    for (pos, player) in assignments {
        squad.push(MatchPlayer::from_player(team_id, player, pos, false));
        used_ids.push(player.id);
        selected_players.push(player);
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
                debug!(
                    "Emergency fill: using {} as outfield player",
                    player.full_name
                );
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
    policy: SelectionPolicy,
) -> Vec<MatchPlayer> {
    let mut subs: Vec<MatchPlayer> = Vec::with_capacity(helpers::DEFAULT_BENCH_SIZE);
    let mut used_ids: Vec<u32> = Vec::new();

    // 1. Backup goalkeeper
    if let Some(gk) = pick_best_goalkeeper(
        remaining,
        &used_ids,
        engine,
        staff,
        is_friendly,
        match_importance,
    ) {
        subs.push(MatchPlayer::from_player(
            team_id,
            gk,
            PlayerPositionType::Goalkeeper,
            false,
        ));
        used_ids.push(gk.id);
    }

    // 2. Role coverage. Real benches are selected for match options, not just
    // broad DEF/MID/FWD buckets.
    for role in bench_plan(tactics, policy) {
        if subs.len() >= helpers::DEFAULT_BENCH_SIZE {
            break;
        }

        let best = remaining
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = bench_role_score(
                    a,
                    role,
                    staff,
                    tactics,
                    engine,
                    date,
                    is_friendly,
                    match_importance,
                    policy,
                );
                let sb = bench_role_score(
                    b,
                    role,
                    staff,
                    tactics,
                    engine,
                    date,
                    is_friendly,
                    match_importance,
                    policy,
                );
                sa.partial_cmp(&sb).unwrap_or(std::cmp::Ordering::Equal)
            })
            .copied();

        if let Some(player) = best {
            if bench_role_fit(player, role, tactics) < 0.25
                && remaining.len() > helpers::DEFAULT_BENCH_SIZE
            {
                continue;
            }
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
                let sa = bench_role_score(
                    a,
                    BenchRole::Impact,
                    staff,
                    tactics,
                    engine,
                    date,
                    is_friendly,
                    match_importance,
                    policy,
                );
                let sb = bench_role_score(
                    b,
                    BenchRole::Impact,
                    staff,
                    tactics,
                    engine,
                    date,
                    is_friendly,
                    match_importance,
                    policy,
                );
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

fn assign_outfield_slots<'p>(
    available: &[&'p Player],
    used_ids: &[u32],
    slots: &[PlayerPositionType],
    staff: &Staff,
    tactics: &Tactics,
    engine: &ScoringEngine,
    date: NaiveDate,
    is_friendly: bool,
    match_importance: f32,
    policy: SelectionPolicy,
) -> Vec<(PlayerPositionType, &'p Player)> {
    if slots.is_empty() {
        return Vec::new();
    }

    let players: Vec<&Player> = available
        .iter()
        .filter(|p| !used_ids.contains(&p.id))
        .filter(|p| !p.positions.is_goalkeeper())
        .copied()
        .collect();

    if players.len() < slots.len() {
        return Vec::new();
    }

    let slot_count = slots.len();
    let full_mask = (1usize << slot_count) - 1;
    let neg_inf = f32::NEG_INFINITY;
    let mut dp = vec![vec![neg_inf; full_mask + 1]; players.len() + 1];
    let mut prev = vec![vec![None; full_mask + 1]; players.len() + 1];
    dp[0][0] = 0.0;

    for (i, player) in players.iter().enumerate() {
        for mask in 0..=full_mask {
            let current = dp[i][mask];
            if !current.is_finite() {
                continue;
            }

            if current > dp[i + 1][mask] {
                dp[i + 1][mask] = current;
                prev[i + 1][mask] = Some((mask, None));
            }

            for (slot_idx, &slot) in slots.iter().enumerate() {
                let bit = 1usize << slot_idx;
                if mask & bit != 0 {
                    continue;
                }
                let score = starting_slot_score(
                    player,
                    slot,
                    staff,
                    tactics,
                    engine,
                    date,
                    is_friendly,
                    match_importance,
                    policy,
                );
                let new_mask = mask | bit;
                let candidate = current + score;
                if candidate > dp[i + 1][new_mask] {
                    dp[i + 1][new_mask] = candidate;
                    prev[i + 1][new_mask] = Some((mask, Some(slot_idx)));
                }
            }
        }
    }

    if !dp[players.len()][full_mask].is_finite() {
        return Vec::new();
    }

    let mut assigned: Vec<Option<&Player>> = vec![None; slot_count];
    let mut mask = full_mask;
    for i in (1..=players.len()).rev() {
        let Some((previous_mask, selected_slot)) = prev[i][mask] else {
            break;
        };
        if let Some(slot_idx) = selected_slot {
            assigned[slot_idx] = Some(players[i - 1]);
        }
        mask = previous_mask;
    }

    assigned
        .into_iter()
        .enumerate()
        .filter_map(|(idx, player)| player.map(|p| (slots[idx], p)))
        .collect()
}

fn starting_slot_score(
    player: &Player,
    slot: PlayerPositionType,
    staff: &Staff,
    tactics: &Tactics,
    engine: &ScoringEngine,
    date: NaiveDate,
    is_friendly: bool,
    match_importance: f32,
    policy: SelectionPolicy,
) -> f32 {
    let target_group = slot.position_group();
    engine.score_player_for_slot(
        player,
        slot,
        target_group,
        staff,
        tactics,
        date,
        is_friendly,
        &[],
    ) + engine.development_minutes_bonus(player, match_importance)
        + engine.fatigue_penalty(player, is_friendly)
        - engine.injury_risk_penalty(player, match_importance, is_friendly)
        + policy_starting_adjustment(player, date, match_importance, policy)
}

fn policy_starting_adjustment(
    player: &Player,
    date: NaiveDate,
    match_importance: f32,
    policy: SelectionPolicy,
) -> f32 {
    let age = DateUtils::age(player.birth_date, date);
    let is_key_player = player
        .contract
        .as_ref()
        .map(|c| {
            matches!(
                c.squad_status,
                PlayerSquadStatus::KeyPlayer | PlayerSquadStatus::FirstTeamRegular
            )
        })
        .unwrap_or(false);
    let is_development_age = age <= 21;
    let idle = player.player_attributes.days_since_last_match as f32;
    // Use position-weighted physical_load so a 90-min wingback gets
    // rotated where a 90-min keeper isn't. Falls back to minutes_last_7
    // when the new field hasn't accumulated yet (early sim, fresh save).
    let load = player
        .load
        .physical_load_7
        .max(player.load.minutes_last_7 * 0.95);
    let morale = player.happiness.morale;

    (match policy {
        SelectionPolicy::BestEleven => {
            let experience = if age >= 24 { 0.35 } else { -0.15 };
            let morale_bonus = ((morale - 50.0) / 50.0).clamp(-1.0, 1.0) * 0.35;
            experience + morale_bonus
        }
        SelectionPolicy::StrongWithRotation => {
            let rest_need = if load > 360.0 { -0.8 } else { 0.0 };
            let fresh_regular = if is_key_player && idle >= 5.0 {
                0.35
            } else {
                0.0
            };
            rest_need + fresh_regular
        }
        SelectionPolicy::ManagedMinutes => {
            let underplayed = (idle / 18.0).min(1.0) * 0.9;
            let fatigue = if load > 300.0 { -1.2 } else { 0.0 };
            underplayed + fatigue
        }
        SelectionPolicy::CupRotation => {
            let underplayed = (idle / 14.0).min(1.0) * 1.6;
            let youth = if is_development_age { 0.9 } else { 0.0 };
            let protect_star = if is_key_player && load > 180.0 {
                -1.4
            } else {
                0.0
            };
            underplayed + youth + protect_star
        }
        SelectionPolicy::YouthDevelopment => {
            let youth = if is_development_age {
                2.0
            } else if age <= 23 {
                1.0
            } else {
                0.0
            };
            let key_player_rest = if is_key_player { -1.2 } else { 0.0 };
            youth + key_player_rest + (idle / 21.0).min(1.0)
        }
    }) * (1.0 - match_importance * 0.25)
}

#[derive(Debug, Clone, Copy)]
enum BenchRole {
    DefensiveCover,
    MidfieldControl,
    Creator,
    WideOption,
    Striker,
    Utility,
    Impact,
    Prospect,
}

fn bench_plan(tactics: &Tactics, policy: SelectionPolicy) -> Vec<BenchRole> {
    let uses_wide_players = tactics.positions().iter().any(|p| {
        matches!(
            p,
            PlayerPositionType::DefenderLeft
                | PlayerPositionType::DefenderRight
                | PlayerPositionType::WingbackLeft
                | PlayerPositionType::WingbackRight
                | PlayerPositionType::MidfielderLeft
                | PlayerPositionType::MidfielderRight
                | PlayerPositionType::AttackingMidfielderLeft
                | PlayerPositionType::AttackingMidfielderRight
                | PlayerPositionType::ForwardLeft
                | PlayerPositionType::ForwardRight
        )
    });

    let mut roles = vec![
        BenchRole::DefensiveCover,
        BenchRole::MidfieldControl,
        BenchRole::Creator,
    ];
    if uses_wide_players {
        roles.push(BenchRole::WideOption);
    } else {
        roles.push(BenchRole::Utility);
    }
    roles.push(BenchRole::Striker);
    roles.push(BenchRole::Impact);
    roles.push(match policy {
        SelectionPolicy::CupRotation | SelectionPolicy::YouthDevelopment => BenchRole::Prospect,
        _ => BenchRole::Utility,
    });
    roles
}

fn bench_role_score(
    player: &Player,
    role: BenchRole,
    staff: &Staff,
    tactics: &Tactics,
    engine: &ScoringEngine,
    date: NaiveDate,
    is_friendly: bool,
    match_importance: f32,
    policy: SelectionPolicy,
) -> f32 {
    engine.overall_quality(player, staff, tactics, date, is_friendly)
        + engine.development_minutes_bonus(player, match_importance)
        + engine.fatigue_penalty(player, is_friendly) * 0.5
        - engine.injury_risk_penalty(player, match_importance, is_friendly) * 0.35
        + bench_role_fit(player, role, tactics) * 4.0
        + bench_policy_adjustment(player, date, policy)
}

fn bench_role_fit(player: &Player, role: BenchRole, tactics: &Tactics) -> f32 {
    let positions = player.positions.positions();
    let has = |pos: PlayerPositionType| positions.contains(&pos);
    let has_any = |targets: &[PlayerPositionType]| targets.iter().any(|&p| has(p));

    match role {
        BenchRole::DefensiveCover => {
            let center = has_any(&[
                PlayerPositionType::DefenderCenter,
                PlayerPositionType::DefenderCenterLeft,
                PlayerPositionType::DefenderCenterRight,
                PlayerPositionType::DefensiveMidfielder,
            ]);
            let wide = has_any(&[
                PlayerPositionType::DefenderLeft,
                PlayerPositionType::DefenderRight,
                PlayerPositionType::WingbackLeft,
                PlayerPositionType::WingbackRight,
            ]);
            if center && wide {
                1.0
            } else if center || wide {
                0.75
            } else {
                0.0
            }
        }
        BenchRole::MidfieldControl => {
            if has_any(&[
                PlayerPositionType::DefensiveMidfielder,
                PlayerPositionType::MidfielderCenter,
                PlayerPositionType::MidfielderCenterLeft,
                PlayerPositionType::MidfielderCenterRight,
            ]) {
                1.0
            } else {
                0.0
            }
        }
        BenchRole::Creator => {
            let positional = has_any(&[
                PlayerPositionType::AttackingMidfielderCenter,
                PlayerPositionType::AttackingMidfielderLeft,
                PlayerPositionType::AttackingMidfielderRight,
                PlayerPositionType::MidfielderCenter,
            ]);
            let creative = (player.skills.mental.vision
                + player.skills.technical.passing
                + player.skills.technical.technique)
                / 60.0;
            if positional {
                creative.max(0.45)
            } else {
                creative * 0.5
            }
        }
        BenchRole::WideOption => {
            if has_any(&[
                PlayerPositionType::DefenderLeft,
                PlayerPositionType::DefenderRight,
                PlayerPositionType::WingbackLeft,
                PlayerPositionType::WingbackRight,
                PlayerPositionType::MidfielderLeft,
                PlayerPositionType::MidfielderRight,
                PlayerPositionType::AttackingMidfielderLeft,
                PlayerPositionType::AttackingMidfielderRight,
                PlayerPositionType::ForwardLeft,
                PlayerPositionType::ForwardRight,
            ]) {
                1.0
            } else {
                0.0
            }
        }
        BenchRole::Striker => {
            if has_any(&[
                PlayerPositionType::Striker,
                PlayerPositionType::ForwardCenter,
                PlayerPositionType::ForwardLeft,
                PlayerPositionType::ForwardRight,
            ]) {
                1.0
            } else {
                0.0
            }
        }
        BenchRole::Utility => {
            let covered = tactics
                .positions()
                .iter()
                .filter(|&&pos| {
                    pos != PlayerPositionType::Goalkeeper && player.positions.get_level(pos) > 0
                })
                .count();
            (covered as f32 / 3.0).clamp(0.0, 1.0)
        }
        BenchRole::Impact => {
            let attacking = (player.skills.technical.dribbling
                + player.skills.technical.finishing
                + player.skills.mental.flair
                + player.skills.physical.pace)
                / 80.0;
            attacking.clamp(0.0, 1.0)
        }
        BenchRole::Prospect => {
            let age = DateUtils::age(player.birth_date, chrono::Utc::now().date_naive());
            if age <= 19 {
                1.0
            } else if age <= 22 {
                0.65
            } else {
                0.0
            }
        }
    }
}

fn bench_policy_adjustment(player: &Player, date: NaiveDate, policy: SelectionPolicy) -> f32 {
    let age = DateUtils::age(player.birth_date, date);
    match policy {
        SelectionPolicy::BestEleven | SelectionPolicy::StrongWithRotation => 0.0,
        SelectionPolicy::ManagedMinutes => {
            (player.player_attributes.days_since_last_match as f32 / 21.0).min(1.0) * 0.7
        }
        SelectionPolicy::CupRotation => {
            let youth = if age <= 21 { 0.8 } else { 0.0 };
            youth + (player.player_attributes.days_since_last_match as f32 / 14.0).min(1.0)
        }
        SelectionPolicy::YouthDevelopment => {
            if age <= 21 {
                1.6
            } else if age <= 23 {
                0.8
            } else {
                0.0
            }
        }
    }
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
            score_a
                .partial_cmp(&score_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .copied()
}
