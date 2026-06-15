use crate::club::{PlayerFieldPositionGroup, PlayerPositionType};
use crate::r#match::player::MatchPlayer;
use crate::{Player, PlayerStatusType, Staff, Tactics};

use super::helpers;
use super::helpers::KeeperAvailability;
use std::cmp::Ordering;

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

    if let Some(gk) = RotationGoalkeeper::pick(available, &used_ids) {
        squad.push(MatchPlayer::from_player(
            team_id,
            gk,
            PlayerPositionType::Goalkeeper,
            false,
        ));
        used_ids.push(gk.id);
    } else if let Some(any) = helpers::pick_best_unused(available, &used_ids) {
        squad.push(MatchPlayer::from_player(
            team_id,
            any,
            PlayerPositionType::Goalkeeper,
            false,
        ));
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
            .filter(|p| !p.positions.is_goalkeeper())
            .max_by(|a, b| {
                let sa = rotation_score_for_slot(a, pos, target_group, staff, tactics);
                let sb = rotation_score_for_slot(b, pos, target_group, staff, tactics);
                sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
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
            .filter(|p| !p.positions.is_goalkeeper())
            .max_by(|a, b| {
                let sa = rotation_overall_quality(a);
                let sb = rotation_overall_quality(b);
                sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
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

    if let Some(gk) = RotationGoalkeeper::pick(remaining, &used_ids) {
        subs.push(MatchPlayer::from_player(
            team_id,
            gk,
            PlayerPositionType::Goalkeeper,
            false,
        ));
        used_ids.push(gk.id);
    }

    while subs.len() < helpers::DEFAULT_BENCH_SIZE {
        let best = remaining
            .iter()
            .filter(|p| !used_ids.contains(&p.id))
            .max_by(|a, b| {
                let sa = rotation_overall_quality(a);
                let sb = rotation_overall_quality(b);
                sa.partial_cmp(&sb).unwrap_or(Ordering::Equal)
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
    slot_group: PlayerFieldPositionGroup,
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

    // Rotation/friendly want-away semantics: keep a listed player sharp, but
    // protect a near-sold one from a meaningless game.
    score += RotationWantAway::adjustment(player);

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
        + RotationWantAway::adjustment(player)
}

/// Want-away nudge for rotation / friendly selection. Unlike the competitive
/// path it has no disaffection arm — a friendly is exactly where a listed
/// player should get minutes to stay sharp. It only (a) gives a small
/// keep-sharp pull to a listed / transfer-requested / unhappy player with no
/// imminent move, and (b) protects a near-transfer (`Bid`/`Trn`) player from
/// injury in a meaningless game.
struct RotationWantAway;

impl RotationWantAway {
    /// Small keep-sharp pull for a want-away player getting rotation minutes.
    const KEEP_SHARP: f32 = 2.0;
    /// Protection strong enough to bench a near-sold player from a rotation /
    /// friendly XI whenever there is anyone else to field.
    const PROTECT_NEAR_TRANSFER: f32 = -12.0;

    fn adjustment(player: &Player) -> f32 {
        let statuses = player.statuses.get();
        if statuses.contains(&PlayerStatusType::Trn) || statuses.contains(&PlayerStatusType::Bid) {
            return Self::PROTECT_NEAR_TRANSFER;
        }
        let want_away = statuses.contains(&PlayerStatusType::Lst)
            || statuses.contains(&PlayerStatusType::Req)
            || statuses.contains(&PlayerStatusType::Unh);
        if want_away {
            Self::KEEP_SHARP
        } else {
            0.0
        }
    }
}

/// Rotation goalkeeper selection, wrapped so the preferred-condition threshold
/// and the tie-break order live with the picker.
struct RotationGoalkeeper;

impl RotationGoalkeeper {
    /// Condition at or above which a real goalkeeper is the preferred rotation
    /// pick. A keeper below this but still at [`helpers::HARD_CONDITION_FLOOR`]
    /// is fielded over an emergency outfielder — a tired keeper saves far more
    /// than an outfielder with zeroed goalkeeping skills.
    const PREFERRED_CONDITION: u32 = 20;

    /// Best available rotation keeper. Real, available keepers only — never an
    /// injured / international-duty / banned one (a suspended keeper isn't
    /// handed a rotation start), and never below the hard condition floor
    /// (that's the emergency-outfielder line). Prefers a fresh keeper but fields
    /// a 15-19% one over an outfielder. Tie-break: freshest condition, then most
    /// rested, then highest current ability.
    fn pick<'p>(available: &[&'p Player], used_ids: &[u32]) -> Option<&'p Player> {
        let candidate = |min_condition: u32| -> Option<&'p Player> {
            available
                .iter()
                .filter(|p| !used_ids.contains(&p.id))
                .filter(|p| KeeperAvailability::is_fallback_available(p, false))
                .filter(|p| p.player_attributes.condition_percentage() >= min_condition)
                .max_by(|a, b| {
                    a.player_attributes
                        .condition_percentage()
                        .cmp(&b.player_attributes.condition_percentage())
                        .then_with(|| {
                            a.player_attributes
                                .days_since_last_match
                                .cmp(&b.player_attributes.days_since_last_match)
                        })
                        .then_with(|| {
                            a.player_attributes
                                .current_ability
                                .cmp(&b.player_attributes.current_ability)
                        })
                })
                .copied()
        };
        candidate(Self::PREFERRED_CONDITION).or_else(|| candidate(helpers::HARD_CONDITION_FLOOR))
    }
}
