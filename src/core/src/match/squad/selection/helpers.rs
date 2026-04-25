use crate::club::{PlayerFieldPositionGroup, PlayerPositionType};
use crate::PlayerPreferredFoot;
use crate::{Player, PlayerStatusType, TacticalStyle, Tactics};

pub const DEFAULT_SQUAD_SIZE: usize = 11;
pub const DEFAULT_BENCH_SIZE: usize = 7;

/// Minimum condition to be physically able to play (15%).
pub const HARD_CONDITION_FLOOR: u32 = 15;

pub fn is_available(player: &Player, is_friendly: bool) -> bool {
    if player.player_attributes.is_injured {
        return false;
    }
    if player.statuses.get().contains(&PlayerStatusType::Int) {
        return false;
    }

    if player.player_attributes.condition_percentage() < HARD_CONDITION_FLOOR {
        return false;
    }

    if !is_friendly {
        if player.player_attributes.is_banned {
            return false;
        }
    }

    true
}

pub fn is_adjacent_group(a: PlayerFieldPositionGroup, b: PlayerFieldPositionGroup) -> bool {
    matches!(
        (a, b),
        (
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder
        ) | (
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Defender
        ) | (
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward
        ) | (
            PlayerFieldPositionGroup::Forward,
            PlayerFieldPositionGroup::Midfielder
        )
    )
}

/// Calculate how well a player fits a target position (0..20)
pub fn position_fit_score(
    player: &Player,
    slot_position: PlayerPositionType,
    slot_group: PlayerFieldPositionGroup,
) -> f32 {
    let exact_level = player.positions.get_level(slot_position);
    if exact_level > 0 {
        return exact_level as f32;
    }

    let player_group = player.position().position_group();

    if player_group == slot_group {
        let primary_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0);
        return primary_level as f32 * same_group_fit_multiplier(player.position(), slot_position);
    }

    let adjacent = matches!(
        (player_group, slot_group),
        (
            PlayerFieldPositionGroup::Defender,
            PlayerFieldPositionGroup::Midfielder
        ) | (
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Defender
        ) | (
            PlayerFieldPositionGroup::Midfielder,
            PlayerFieldPositionGroup::Forward
        ) | (
            PlayerFieldPositionGroup::Forward,
            PlayerFieldPositionGroup::Midfielder
        )
    );

    if adjacent {
        let primary_level = player
            .positions
            .positions
            .iter()
            .map(|p| p.level)
            .max()
            .unwrap_or(0);
        return primary_level as f32 * 0.4;
    }

    1.0
}

/// More realistic compatibility for nearby roles. This keeps the old broad
/// group fallback, but distinguishes sensible conversions (DR -> WBR) from
/// desperate ones (DC -> DL).
fn same_group_fit_multiplier(primary: PlayerPositionType, slot: PlayerPositionType) -> f32 {
    if primary == slot {
        return 1.0;
    }

    match (primary, slot) {
        (PlayerPositionType::DefenderLeft, PlayerPositionType::WingbackLeft)
        | (PlayerPositionType::WingbackLeft, PlayerPositionType::DefenderLeft)
        | (PlayerPositionType::DefenderRight, PlayerPositionType::WingbackRight)
        | (PlayerPositionType::WingbackRight, PlayerPositionType::DefenderRight)
        | (PlayerPositionType::MidfielderLeft, PlayerPositionType::AttackingMidfielderLeft)
        | (PlayerPositionType::AttackingMidfielderLeft, PlayerPositionType::MidfielderLeft)
        | (PlayerPositionType::MidfielderRight, PlayerPositionType::AttackingMidfielderRight)
        | (PlayerPositionType::AttackingMidfielderRight, PlayerPositionType::MidfielderRight) => {
            0.86
        }

        (PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderCenterLeft)
        | (PlayerPositionType::DefenderCenter, PlayerPositionType::DefenderCenterRight)
        | (PlayerPositionType::DefenderCenterLeft, PlayerPositionType::DefenderCenter)
        | (PlayerPositionType::DefenderCenterRight, PlayerPositionType::DefenderCenter)
        | (PlayerPositionType::MidfielderCenter, PlayerPositionType::MidfielderCenterLeft)
        | (PlayerPositionType::MidfielderCenter, PlayerPositionType::MidfielderCenterRight)
        | (PlayerPositionType::MidfielderCenterLeft, PlayerPositionType::MidfielderCenter)
        | (PlayerPositionType::MidfielderCenterRight, PlayerPositionType::MidfielderCenter)
        | (PlayerPositionType::ForwardCenter, PlayerPositionType::Striker)
        | (PlayerPositionType::Striker, PlayerPositionType::ForwardCenter) => 0.82,

        (PlayerPositionType::ForwardLeft, PlayerPositionType::AttackingMidfielderLeft)
        | (PlayerPositionType::AttackingMidfielderLeft, PlayerPositionType::ForwardLeft)
        | (PlayerPositionType::ForwardRight, PlayerPositionType::AttackingMidfielderRight)
        | (PlayerPositionType::AttackingMidfielderRight, PlayerPositionType::ForwardRight) => 0.78,

        _ => 0.62,
    }
}

pub fn side_foot_bonus(player: &Player, position: PlayerPositionType) -> f32 {
    match position {
        PlayerPositionType::DefenderLeft
        | PlayerPositionType::DefenderCenterLeft
        | PlayerPositionType::WingbackLeft
        | PlayerPositionType::MidfielderLeft
        | PlayerPositionType::AttackingMidfielderLeft
        | PlayerPositionType::ForwardLeft => match player.preferred_foot {
            PlayerPreferredFoot::Left | PlayerPreferredFoot::Both => 0.45,
            PlayerPreferredFoot::Right => -0.25,
        },
        PlayerPositionType::DefenderRight
        | PlayerPositionType::DefenderCenterRight
        | PlayerPositionType::WingbackRight
        | PlayerPositionType::MidfielderRight
        | PlayerPositionType::AttackingMidfielderRight
        | PlayerPositionType::ForwardRight => match player.preferred_foot {
            PlayerPreferredFoot::Right | PlayerPreferredFoot::Both => 0.45,
            PlayerPreferredFoot::Left => -0.25,
        },
        _ => 0.0,
    }
}

/// Tactical style bonus for a player in a given position
pub fn tactical_style_bonus(
    player: &Player,
    position: PlayerPositionType,
    tactics: &Tactics,
) -> f32 {
    let mut bonus = 0.0;

    match tactics.tactical_style() {
        TacticalStyle::Attacking => {
            if position.is_forward() || position == PlayerPositionType::AttackingMidfielderCenter {
                bonus += player.skills.technical.finishing * 0.1;
                bonus += player.skills.mental.off_the_ball * 0.1;
            }
        }
        TacticalStyle::Defensive => {
            if position.is_defender() || position == PlayerPositionType::DefensiveMidfielder {
                bonus += player.skills.technical.tackling * 0.1;
                bonus += player.skills.mental.positioning * 0.1;
            }
        }
        TacticalStyle::Possession => {
            bonus += player.skills.technical.passing * 0.08;
            bonus += player.skills.mental.vision * 0.08;
        }
        TacticalStyle::Counterattack => {
            if position.is_forward() || position.is_midfielder() {
                bonus += player.skills.physical.pace * 0.1;
                bonus += player.skills.mental.off_the_ball * 0.08;
            }
        }
        TacticalStyle::WingPlay | TacticalStyle::WidePlay => {
            if position == PlayerPositionType::WingbackLeft
                || position == PlayerPositionType::WingbackRight
                || position == PlayerPositionType::MidfielderLeft
                || position == PlayerPositionType::MidfielderRight
            {
                bonus += player.skills.technical.crossing * 0.1;
                bonus += player.skills.physical.pace * 0.08;
            }
        }
        _ => {}
    }

    bonus
}

/// Find the best tactical position for a player within the formation
pub fn best_tactical_position(player: &Player, tactics: &Tactics) -> PlayerPositionType {
    let player_group = player.position().position_group();

    for &pos in tactics.positions() {
        if player.positions.get_level(pos) > 0 {
            return pos;
        }
    }

    for &pos in tactics.positions() {
        if pos.position_group() == player_group && pos != PlayerPositionType::Goalkeeper {
            return pos;
        }
    }

    for &pos in tactics.positions() {
        if pos != PlayerPositionType::Goalkeeper {
            return pos;
        }
    }

    player.position()
}

pub fn pick_best_unused<'p>(available: &[&'p Player], used_ids: &[u32]) -> Option<&'p Player> {
    available
        .iter()
        .filter(|p| !used_ids.contains(&p.id))
        .max_by(|a, b| {
            let sa = a.player_attributes.current_ability;
            let sb = b.player_attributes.current_ability;
            sa.cmp(&sb)
        })
        .copied()
}
