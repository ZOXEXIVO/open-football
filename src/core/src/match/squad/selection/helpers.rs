use crate::club::{PlayerFieldPositionGroup, PlayerPositionType};
use crate::{Player, PlayerPreferredFoot, PlayerStatusType, TacticalStyle, Tactics};

pub const DEFAULT_SQUAD_SIZE: usize = 11;
pub const DEFAULT_BENCH_SIZE: usize = 7;

/// Minimum condition to be physically able to play (15%).
pub const HARD_CONDITION_FLOOR: u32 = 15;

/// Selection-time status census for diagnostics. Keeps genuine match
/// *unavailability* (injury, ban, international duty, low condition) strictly
/// separate from *market* status (listed, loan-listed, requested, unhappy) and
/// *near-transfer* status (bid accepted, agreed move). Market and near-transfer
/// statuses never make a player unavailable — they only colour selection — so
/// grouping them beside the unavailable causes (as the old debug line did) is
/// misleading when debugging a short squad.
#[derive(Debug, Default, Clone, Copy)]
pub struct SelectionStatusCensus {
    // ── genuine availability blocks ──
    pub injured: usize,
    pub international: usize,
    pub low_condition: usize,
    pub banned: usize,
    // ── market statuses (still selectable) ──
    pub listed: usize,
    pub loan_listed: usize,
    pub requested: usize,
    pub unhappy: usize,
    // ── near-transfer statuses (still selectable) ──
    pub bid_accepted: usize,
    pub agreed_transfer: usize,
}

impl SelectionStatusCensus {
    /// Census the squad. `is_friendly` mirrors the availability gate — a ban
    /// doesn't apply in a friendly, so it isn't counted as unavailable there.
    pub fn of(players: &[&Player], is_friendly: bool) -> Self {
        let mut c = SelectionStatusCensus::default();
        for p in players {
            if p.player_attributes.is_injured {
                c.injured += 1;
            }
            if p.statuses.is_on_international_duty() {
                c.international += 1;
            }
            if !p.player_attributes.is_injured
                && p.player_attributes.condition_percentage() < HARD_CONDITION_FLOOR
            {
                c.low_condition += 1;
            }
            if !is_friendly && p.player_attributes.is_banned {
                c.banned += 1;
            }
            let s = p.statuses.get();
            if s.contains(&PlayerStatusType::Lst) {
                c.listed += 1;
            }
            if s.contains(&PlayerStatusType::Loa) {
                c.loan_listed += 1;
            }
            if s.contains(&PlayerStatusType::Req) {
                c.requested += 1;
            }
            if s.contains(&PlayerStatusType::Unh) {
                c.unhappy += 1;
            }
            if s.contains(&PlayerStatusType::Bid) {
                c.bid_accepted += 1;
            }
            if s.contains(&PlayerStatusType::Trn) {
                c.agreed_transfer += 1;
            }
        }
        c
    }

    /// Players blocked from selection by a genuine availability cause. Market
    /// and near-transfer statuses are deliberately excluded.
    pub fn unavailable_total(&self) -> usize {
        self.injured + self.international + self.low_condition + self.banned
    }

    /// Players carrying a market or near-transfer status. These remain fully
    /// selectable — the figure is reported separately from unavailability.
    pub fn market_total(&self) -> usize {
        self.listed
            + self.loan_listed
            + self.requested
            + self.unhappy
            + self.bid_accepted
            + self.agreed_transfer
    }
}

pub struct PlayerAvailability;

impl PlayerAvailability {
    /// Match availability is a *physical/legal* question only: injury,
    /// international duty, a hard condition floor, and (in competitive games) a
    /// ban. Transfer-market status is deliberately NOT consulted here — a
    /// transfer-listed (`Lst`), transfer-requested (`Req`) or unhappy (`Unh`)
    /// player is still a contracted club asset and remains fully selectable.
    /// Those signals shape *how* the manager picks him (see
    /// `ScoringEngine::want_away_adjustment`), never *whether* he can play.
    pub fn is_available(player: &Player, is_friendly: bool) -> bool {
        if player.player_attributes.is_injured {
            return false;
        }
        if player.statuses.is_on_international_duty() {
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
}

pub struct KeeperAvailability;

impl KeeperAvailability {
    /// Keeper-fallback availability. When the normal [`is_available`] filter has
    /// excluded every goalkeeper on the roster, the selector would otherwise press
    /// an outfielder — whose goalkeeping skills default to zero — into goal. A
    /// low-condition keeper still saves far more, so the fallback re-admits keepers
    /// who are physically able to play even below [`HARD_CONDITION_FLOOR`]. It never
    /// re-admits one who is injured, away on international duty, or (in a
    /// competitive match) banned; the force-selection / non-Main-team pin stays with
    /// the caller.
    pub fn is_fallback_available(player: &Player, is_friendly: bool) -> bool {
        if !player.positions.is_goalkeeper() {
            return false;
        }
        if player.player_attributes.is_injured {
            return false;
        }
        if player.statuses.is_on_international_duty() {
            return false;
        }
        if !is_friendly && player.player_attributes.is_banned {
            return false;
        }
        true
    }
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
