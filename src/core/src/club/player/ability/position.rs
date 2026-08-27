use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter, Result};

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, PartialOrd, Serialize, Deserialize)]
pub enum PlayerPositionType {
    Goalkeeper,
    Sweeper,
    DefenderLeft,
    DefenderCenterLeft,
    DefenderCenter,
    DefenderCenterRight,
    DefenderRight,
    DefensiveMidfielder,
    MidfielderLeft,
    MidfielderCenterLeft,
    MidfielderCenter,
    MidfielderCenterRight,
    MidfielderRight,
    AttackingMidfielderLeft,
    AttackingMidfielderCenter,
    AttackingMidfielderRight,
    WingbackLeft,
    WingbackRight,
    Striker,
    ForwardLeft,
    ForwardCenter,
    ForwardRight,
}

impl Display for PlayerPositionType {
    fn fmt(&self, f: &mut Formatter) -> Result {
        write!(f, "{:?}", self)
    }
}

impl PlayerPositionType {
    /// Every role in the game, in declaration order. Backs the
    /// group-coverage masks and anything else that has to enumerate the
    /// position space without hand-listing it a second time.
    pub const ALL: [PlayerPositionType; 22] = [
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::Sweeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenter,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::DefensiveMidfielder,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenter,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::AttackingMidfielderLeft,
        PlayerPositionType::AttackingMidfielderCenter,
        PlayerPositionType::AttackingMidfielderRight,
        PlayerPositionType::WingbackLeft,
        PlayerPositionType::WingbackRight,
        PlayerPositionType::Striker,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardCenter,
        PlayerPositionType::ForwardRight,
    ];

    pub fn as_i18n_key(&self) -> &'static str {
        match *self {
            PlayerPositionType::Goalkeeper => "pos_goalkeeper",
            PlayerPositionType::Sweeper => "pos_sweeper",
            PlayerPositionType::DefenderLeft => "pos_defender_left",
            PlayerPositionType::DefenderCenterLeft => "pos_defender_center_left",
            PlayerPositionType::DefenderCenter => "pos_defender_center",
            PlayerPositionType::DefenderCenterRight => "pos_defender_center_right",
            PlayerPositionType::DefenderRight => "pos_defender_right",
            PlayerPositionType::DefensiveMidfielder => "pos_defensive_midfielder",
            PlayerPositionType::MidfielderLeft => "pos_midfielder_left",
            PlayerPositionType::MidfielderCenterLeft => "pos_midfielder_center_left",
            PlayerPositionType::MidfielderCenter => "pos_midfielder_center",
            PlayerPositionType::MidfielderCenterRight => "pos_midfielder_center_right",
            PlayerPositionType::MidfielderRight => "pos_midfielder_right",
            PlayerPositionType::AttackingMidfielderLeft => "pos_attacking_midfielder_left",
            PlayerPositionType::AttackingMidfielderCenter => "pos_attacking_midfielder_center",
            PlayerPositionType::AttackingMidfielderRight => "pos_attacking_midfielder_right",
            PlayerPositionType::WingbackLeft => "pos_wingback_left",
            PlayerPositionType::WingbackRight => "pos_wingback_right",
            PlayerPositionType::ForwardLeft => "pos_forward_left",
            PlayerPositionType::ForwardCenter => "pos_forward_center",
            PlayerPositionType::ForwardRight => "pos_forward_right",
            PlayerPositionType::Striker => "pos_striker",
        }
    }

    #[inline]
    pub fn get_short_name(&self) -> &'static str {
        match *self {
            PlayerPositionType::Goalkeeper => "GK",
            PlayerPositionType::Sweeper => "SW",
            PlayerPositionType::DefenderLeft => "DL",
            PlayerPositionType::DefenderCenterLeft => "DCL",
            PlayerPositionType::DefenderCenter => "DC",
            PlayerPositionType::DefenderCenterRight => "DCR",
            PlayerPositionType::DefenderRight => "DR",
            PlayerPositionType::DefensiveMidfielder => "DM",
            PlayerPositionType::MidfielderLeft => "ML",
            PlayerPositionType::MidfielderCenterLeft => "MCL",
            PlayerPositionType::MidfielderCenter => "MC",
            PlayerPositionType::MidfielderCenterRight => "MCR",
            PlayerPositionType::MidfielderRight => "MR",
            PlayerPositionType::AttackingMidfielderLeft => "AML",
            PlayerPositionType::AttackingMidfielderCenter => "AMC",
            PlayerPositionType::AttackingMidfielderRight => "AMR",
            PlayerPositionType::WingbackLeft => "WL",
            PlayerPositionType::WingbackRight => "WR",
            PlayerPositionType::ForwardLeft => "FL",
            PlayerPositionType::ForwardCenter => "FC",
            PlayerPositionType::ForwardRight => "FR",
            PlayerPositionType::Striker => "ST",
        }
    }

    #[inline]
    pub fn is_goalkeeper(&self) -> bool {
        self.position_group() == PlayerFieldPositionGroup::Goalkeeper
    }

    #[inline]
    pub fn is_defender(&self) -> bool {
        self.position_group() == PlayerFieldPositionGroup::Defender
    }

    #[inline]
    pub fn is_midfielder(&self) -> bool {
        self.position_group() == PlayerFieldPositionGroup::Midfielder
    }

    #[inline]
    pub fn is_forward(&self) -> bool {
        self.position_group() == PlayerFieldPositionGroup::Forward
    }

    /// True for AML / AMC / AMR. These positions group under
    /// `Midfielder` for shape and selection, but their shooting /
    /// chance-creation expectations are forward-like — the engine's
    /// strict midfielder shot gates suppress them to near-zero G/A
    /// without this carve-out.
    #[inline]
    pub fn is_attacking_midfielder(&self) -> bool {
        matches!(
            self,
            PlayerPositionType::AttackingMidfielderLeft
                | PlayerPositionType::AttackingMidfielderCenter
                | PlayerPositionType::AttackingMidfielderRight
        )
    }

    /// True for central defenders (CB / sweeper) — the players sent up to
    /// attack an attacking corner (they're the aerial threat). Excludes
    /// full-backs / wing-backs, who stay back to cover the counter.
    #[inline]
    pub fn is_central_defender(&self) -> bool {
        matches!(
            self,
            PlayerPositionType::DefenderCenterLeft
                | PlayerPositionType::DefenderCenter
                | PlayerPositionType::DefenderCenterRight
                | PlayerPositionType::Sweeper
        )
    }

    /// True for the central midfield band (CM / AMC) — the players who
    /// make the late central run into the box and arrive for cutbacks.
    /// Deliberately excludes wide mids (ML/MR), wingbacks and wide AMs,
    /// who hold width and deliver crosses rather than arriving centrally.
    /// This is the gate for the single elected "arriving runner" so the
    /// box-run redistribution doesn't pull the whole midfield up.
    #[inline]
    pub fn is_central_midfielder(&self) -> bool {
        matches!(
            self,
            PlayerPositionType::MidfielderCenterLeft
                | PlayerPositionType::MidfielderCenter
                | PlayerPositionType::MidfielderCenterRight
                | PlayerPositionType::AttackingMidfielderCenter
        )
    }

    /// The screen in front of the back four.
    ///
    /// `position_group()` files him under `Defender`, which is right for
    /// duties and wrong for geometry: everything that reasons about "the
    /// back line" has to exclude him, because he is deliberately ten to
    /// fifteen metres in front of it. Both the shape constraint and the
    /// harness's back-line sampler need this distinction and were making
    /// it separately (and inconsistently).
    #[inline]
    pub fn is_defensive_midfielder(&self) -> bool {
        matches!(self, PlayerPositionType::DefensiveMidfielder)
    }

    #[inline]
    pub fn position_group(&self) -> PlayerFieldPositionGroup {
        match *self {
            PlayerPositionType::Goalkeeper => PlayerFieldPositionGroup::Goalkeeper,
            PlayerPositionType::Sweeper
            | PlayerPositionType::DefenderLeft
            | PlayerPositionType::DefenderCenterLeft
            | PlayerPositionType::DefenderCenter
            | PlayerPositionType::DefenderCenterRight
            | PlayerPositionType::DefenderRight
            | PlayerPositionType::DefensiveMidfielder => PlayerFieldPositionGroup::Defender,
            PlayerPositionType::MidfielderLeft
            | PlayerPositionType::MidfielderCenterLeft
            | PlayerPositionType::MidfielderCenter
            | PlayerPositionType::MidfielderCenterRight
            | PlayerPositionType::MidfielderRight
            | PlayerPositionType::AttackingMidfielderLeft
            | PlayerPositionType::AttackingMidfielderCenter
            | PlayerPositionType::AttackingMidfielderRight
            | PlayerPositionType::WingbackLeft
            | PlayerPositionType::WingbackRight => PlayerFieldPositionGroup::Midfielder,
            PlayerPositionType::ForwardLeft
            | PlayerPositionType::ForwardCenter
            | PlayerPositionType::ForwardRight
            | PlayerPositionType::Striker => PlayerFieldPositionGroup::Forward,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerPositions {
    pub positions: Vec<PlayerPosition>,
}

const REQUIRED_POSITION_LEVEL: u8 = 5;

/// How much of a player's ability survives being asked to fill a role.
///
/// One curve for every "could he do a job there?" read in the simulator,
/// so squad planning, the recruitment search and the loan gates can never
/// disagree about what a secondary position is worth. Continuous in the
/// familiarity level rather than a natural / not-natural switch: a 17 is
/// very nearly a natural, a 5 is a body in the right shirt.
pub struct RoleFamiliarity;

impl RoleFamiliarity {
    /// Share of current ability a player carries into a role he holds at
    /// `level`, 0.0..=1.0.
    ///
    /// The bands are the familiarity ladder — natural, accomplished,
    /// competent, unconvincing, awkward — and join up continuously at every
    /// boundary, so nudging a familiarity by one never moves a squad decision
    /// by a cliff.
    ///
    /// Deliberately gentle. This scales ABILITY, which is not the same job as
    /// the tactical fitness curve, where the familiarity term is one weighted
    /// input among three: a competent midfielder asked to play on the left is
    /// still most of the player he is, and pricing that as a third of him off
    /// would have every club in the world reading its own shape as a crisis.
    pub fn credit(level: u8) -> f32 {
        let l = level as f32;
        if level >= 18 {
            1.0
        } else if level >= 15 {
            0.92 + (l - 15.0) * (0.08 / 3.0)
        } else if level >= 12 {
            0.80 + (l - 12.0) * (0.12 / 3.0)
        } else if level >= 8 {
            0.62 + (l - 8.0) * (0.18 / 4.0)
        } else if level >= REQUIRED_POSITION_LEVEL {
            0.50 + (l - REQUIRED_POSITION_LEVEL as f32) * (0.12 / 3.0)
        } else {
            0.0
        }
    }

    /// Best effective ability a player of `current_ability` brings to any
    /// role in `group`, given the familiarity he holds at each. Zero when he
    /// cannot fill the group at all.
    ///
    /// This is the honest answer to "how good are we there?" — a question the
    /// squad-depth and loan gates used to answer by filtering on a player's
    /// single primary label, which silently excluded every man who plays the
    /// role as his second position.
    pub fn best_in_group(
        positions: &PlayerPositions,
        current_ability: u8,
        group: PlayerFieldPositionGroup,
    ) -> u8 {
        Self::effective_ability(current_ability, Self::best_level_in_group(positions, group))
    }

    /// Highest familiarity the player holds at any role in `group`, or zero
    /// when he holds none.
    pub fn best_level_in_group(positions: &PlayerPositions, group: PlayerFieldPositionGroup) -> u8 {
        positions
            .positions
            .iter()
            .filter(|p| p.position.position_group() == group)
            .map(|p| p.level)
            .max()
            .unwrap_or(0)
    }

    /// Effective ability a player of `current_ability` brings to a role he
    /// holds at `level`. Zero familiarity means he cannot fill it at all.
    pub fn effective_ability(current_ability: u8, level: u8) -> u8 {
        (current_ability as f32 * Self::credit(level))
            .round()
            .clamp(0.0, 255.0) as u8
    }
}

/// The set of roles a player can actually fill, as a bitmask over
/// [`PlayerPositionType`].
///
/// The market used to ask a player's single primary label which group he
/// belonged to, which is a different question from "can he play there".
/// More than half of a senior population is multi-position at identical
/// competence, so that label is an arbitrary pick among equals — and asking
/// it who could lead a line hid every wide forward who plays centre-forward
/// from the search. Headcount still belongs to the label (a man occupies one
/// shirt); *quality at a role* and *who to look at* belong here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PositionCoverage(u32);

impl PositionCoverage {
    /// Roles the player holds at playable familiarity. Mirrors
    /// [`PlayerPositions::positions`], including its fallback to the best
    /// listed role for a player whose every entry sits under the bar.
    pub fn of(positions: &PlayerPositions) -> Self {
        let mut mask = 0u32;
        for p in &positions.positions {
            if p.level >= REQUIRED_POSITION_LEVEL {
                mask |= 1 << (p.position as u32);
            }
        }
        if let (0, Some(best)) = (mask, positions.positions.iter().max_by_key(|p| p.level)) {
            mask |= 1 << (best.position as u32);
        }
        PositionCoverage(mask)
    }

    /// Coverage of a single role — the shape a caller who only knows one
    /// position (a synthetic fixture, a legacy adapter) can still build.
    pub fn single(position: PlayerPositionType) -> Self {
        PositionCoverage(1 << (position as u32))
    }

    pub fn covers(&self, position: PlayerPositionType) -> bool {
        self.0 & (1 << (position as u32)) != 0
    }

    /// True when the player can fill any role in `group`.
    pub fn covers_group(&self, group: PlayerFieldPositionGroup) -> bool {
        self.0 & group.coverage_mask() != 0
    }

    pub fn is_empty(&self) -> bool {
        self.0 == 0
    }
}

impl PlayerPositions {
    pub fn positions(&self) -> Vec<PlayerPositionType> {
        let filtered: Vec<PlayerPositionType> = self
            .positions
            .iter()
            .filter(|p| p.level >= REQUIRED_POSITION_LEVEL)
            .map(|p| p.position)
            .collect();

        if filtered.is_empty() {
            self.positions
                .iter()
                .max_by_key(|p| p.level)
                .map(|p| vec![p.position])
                .unwrap_or_default()
        } else {
            filtered
        }
    }

    /// First entry of [`Self::positions`] without building the Vec —
    /// `Player::position()` is the single hottest accessor in the simulator
    /// and used to allocate on every call just to take `.first()`. Same
    /// semantics: first stored position at playable level, else the
    /// max-level fallback (`max_by_key` keeps the LAST maximum on ties,
    /// preserved here exactly).
    pub fn primary(&self) -> Option<PlayerPositionType> {
        if let Some(p) = self
            .positions
            .iter()
            .find(|p| p.level >= REQUIRED_POSITION_LEVEL)
        {
            return Some(p.position);
        }
        self.positions
            .iter()
            .max_by_key(|p| p.level)
            .map(|p| p.position)
    }

    pub fn display_positions(&self) -> Vec<&str> {
        self.positions()
            .iter()
            .map(|p| p.get_short_name())
            .collect()
    }

    pub fn display_positions_compact(&self) -> String {
        let names: Vec<&str> = self.display_positions();
        if names.len() <= 1 {
            return names.join(", ");
        }

        // Group positions by base prefix (e.g. "DC", "MC", "AM", "D", "M", "F", "W")
        // Groups: DC/DCL/DCR, MC/MCL/MCR, AM/AML/AMC/AMR, D/DL/DR, M/ML/MR, F/FL/FC/FR, W/WL/WR
        struct Group {
            base: &'static str,
            center: &'static str,
            left: &'static str,
            right: &'static str,
        }

        const GROUPS: &[Group] = &[
            Group {
                base: "DC",
                center: "DC",
                left: "DCL",
                right: "DCR",
            },
            Group {
                base: "MC",
                center: "MC",
                left: "MCL",
                right: "MCR",
            },
            Group {
                base: "AM",
                center: "AMC",
                left: "AML",
                right: "AMR",
            },
            Group {
                base: "D",
                center: "",
                left: "DL",
                right: "DR",
            },
            Group {
                base: "M",
                center: "",
                left: "ML",
                right: "MR",
            },
            Group {
                base: "F",
                center: "FC",
                left: "FL",
                right: "FR",
            },
            Group {
                base: "W",
                center: "",
                left: "WL",
                right: "WR",
            },
        ];

        let mut used = vec![false; names.len()];
        let mut result: Vec<String> = Vec::new();

        for group in GROUPS {
            let has_center = !group.center.is_empty() && names.iter().any(|n| *n == group.center);
            let has_left = names.iter().any(|n| *n == group.left);
            let has_right = names.iter().any(|n| *n == group.right);

            let count = has_center as u8 + has_left as u8 + has_right as u8;
            if count < 2 {
                continue;
            }

            // Mark used
            for (i, n) in names.iter().enumerate() {
                if (has_center && *n == group.center)
                    || (has_left && *n == group.left)
                    || (has_right && *n == group.right)
                {
                    used[i] = true;
                }
            }

            // Build compact string
            let mut sides = String::new();
            if has_left {
                sides.push('L');
            }
            if has_center && !group.center.is_empty() {
                // For groups where center == base (DC, MC), don't add C inside parens
                if group.center != group.base {
                    sides.push('C');
                }
            }
            if has_right {
                sides.push('R');
            }

            if sides.is_empty() {
                result.push(group.base.to_string());
            } else if has_center && group.center == group.base && !has_left && !has_right {
                result.push(group.base.to_string());
            } else {
                result.push(format!("{}({})", group.base, sides));
            }
        }

        // Add remaining ungrouped positions
        for (i, n) in names.iter().enumerate() {
            if !used[i] {
                result.push(n.to_string());
            }
        }

        result.join(", ")
    }

    /// Membership test against [`Self::positions`] without building the
    /// Vec. Mirrors its two-branch semantics: match among playable-level
    /// entries when any exist, else against the max-level fallback.
    pub fn has_position(&self, position: PlayerPositionType) -> bool {
        let mut any_playable = false;
        for p in &self.positions {
            if p.level >= REQUIRED_POSITION_LEVEL {
                any_playable = true;
                if p.position == position {
                    return true;
                }
            }
        }
        if any_playable {
            return false;
        }
        self.positions
            .iter()
            .max_by_key(|p| p.level)
            .map(|p| p.position == position)
            .unwrap_or(false)
    }

    pub fn is_goalkeeper(&self) -> bool {
        self.has_position(PlayerPositionType::Goalkeeper)
    }

    pub fn get_level(&self, position: PlayerPositionType) -> u8 {
        match self.positions.iter().find(|p| p.position == position) {
            Some(p) => p.level,
            None => 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerPosition {
    pub position: PlayerPositionType,
    pub level: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_position_names_is_correct() {
        assert_eq!("GK", PlayerPositionType::Goalkeeper.get_short_name());
        assert_eq!("SW", PlayerPositionType::Sweeper.get_short_name());
        assert_eq!("DL", PlayerPositionType::DefenderLeft.get_short_name());
        assert_eq!("DC", PlayerPositionType::DefenderCenter.get_short_name());
        assert_eq!("DR", PlayerPositionType::DefenderRight.get_short_name());
        assert_eq!(
            "DM",
            PlayerPositionType::DefensiveMidfielder.get_short_name()
        );
        assert_eq!("ML", PlayerPositionType::MidfielderLeft.get_short_name());
        assert_eq!("MC", PlayerPositionType::MidfielderCenter.get_short_name());
        assert_eq!("MR", PlayerPositionType::MidfielderRight.get_short_name());
        assert_eq!(
            "AML",
            PlayerPositionType::AttackingMidfielderLeft.get_short_name()
        );
        assert_eq!(
            "AMC",
            PlayerPositionType::AttackingMidfielderCenter.get_short_name()
        );
        assert_eq!(
            "AMR",
            PlayerPositionType::AttackingMidfielderRight.get_short_name()
        );
        assert_eq!("ST", PlayerPositionType::Striker.get_short_name());
        assert_eq!("WL", PlayerPositionType::WingbackLeft.get_short_name());
        assert_eq!("WR", PlayerPositionType::WingbackRight.get_short_name());
    }

    #[test]
    fn display_positions_return_with_over_15_level() {
        let positions = PlayerPositions {
            positions: vec![
                PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 1,
                },
                PlayerPosition {
                    position: PlayerPositionType::Sweeper,
                    level: 10,
                },
                PlayerPosition {
                    position: PlayerPositionType::Striker,
                    level: 14,
                },
                PlayerPosition {
                    position: PlayerPositionType::WingbackLeft,
                    level: 15,
                },
                PlayerPosition {
                    position: PlayerPositionType::WingbackRight,
                    level: 20,
                },
            ],
        };

        let display_positions = positions.display_positions().join(",");

        assert_eq!("SW,ST,WL,WR", display_positions);
    }

    fn make_positions(types: &[PlayerPositionType]) -> PlayerPositions {
        PlayerPositions {
            positions: types
                .iter()
                .map(|&t| PlayerPosition {
                    position: t,
                    level: 10,
                })
                .collect(),
        }
    }

    #[test]
    fn compact_mc_mcl_mcr() {
        let p = make_positions(&[
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterLeft,
            PlayerPositionType::MidfielderCenterRight,
        ]);
        assert_eq!("MC(LR)", p.display_positions_compact());
    }

    #[test]
    fn compact_mc_mcr() {
        let p = make_positions(&[
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
        ]);
        assert_eq!("MC(R)", p.display_positions_compact());
    }

    #[test]
    fn compact_dc_dcl_dcr() {
        let p = make_positions(&[
            PlayerPositionType::DefenderCenter,
            PlayerPositionType::DefenderCenterLeft,
            PlayerPositionType::DefenderCenterRight,
        ]);
        assert_eq!("DC(LR)", p.display_positions_compact());
    }

    #[test]
    fn compact_aml_amc_amr() {
        let p = make_positions(&[
            PlayerPositionType::AttackingMidfielderLeft,
            PlayerPositionType::AttackingMidfielderCenter,
            PlayerPositionType::AttackingMidfielderRight,
        ]);
        assert_eq!("AM(LCR)", p.display_positions_compact());
    }

    #[test]
    fn compact_wl_wr() {
        let p = make_positions(&[
            PlayerPositionType::WingbackLeft,
            PlayerPositionType::WingbackRight,
        ]);
        assert_eq!("W(LR)", p.display_positions_compact());
    }

    #[test]
    fn compact_single_position() {
        let p = make_positions(&[PlayerPositionType::Striker]);
        assert_eq!("ST", p.display_positions_compact());
    }

    #[test]
    fn compact_no_grouping_needed() {
        let p = make_positions(&[PlayerPositionType::Goalkeeper, PlayerPositionType::Striker]);
        assert_eq!("GK, ST", p.display_positions_compact());
    }

    #[test]
    fn compact_mixed_grouped_and_ungrouped() {
        let p = make_positions(&[
            PlayerPositionType::MidfielderCenter,
            PlayerPositionType::MidfielderCenterRight,
            PlayerPositionType::Striker,
        ]);
        assert_eq!("MC(R), ST", p.display_positions_compact());
    }
}

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PlayerFieldPositionGroup {
    Goalkeeper,
    Defender,
    Midfielder,
    Forward,
}

impl PlayerFieldPositionGroup {
    /// Total number of position groups — the length of any table indexed
    /// by [`Self::index`].
    pub const COUNT: usize = 4;

    /// The four groups in [`Self::index`] order — for callers that have to
    /// walk every group rather than the one a player is labelled with.
    pub const ALL: [PlayerFieldPositionGroup; Self::COUNT] = [
        PlayerFieldPositionGroup::Goalkeeper,
        PlayerFieldPositionGroup::Defender,
        PlayerFieldPositionGroup::Midfielder,
        PlayerFieldPositionGroup::Forward,
    ];

    /// Bitmask of every [`PlayerPositionType`] in this group, in the layout
    /// [`PositionCoverage`] uses. Built by walking
    /// [`PlayerPositionType::ALL`] so it can never drift from the group
    /// mapping itself.
    pub fn coverage_mask(self) -> u32 {
        let mut mask = 0u32;
        let mut i = 0;
        while i < PlayerPositionType::ALL.len() {
            let position = PlayerPositionType::ALL[i];
            if position.position_group() as u8 == self as u8 {
                mask |= 1 << (position as u32);
            }
            i += 1;
        }
        mask
    }

    /// Stable 0-based index for group-keyed lookup tables (the transfer
    /// pipeline partitions candidate pools per group so per-request scans
    /// don't walk the other three groups).
    pub fn index(&self) -> usize {
        match self {
            PlayerFieldPositionGroup::Goalkeeper => 0,
            PlayerFieldPositionGroup::Defender => 1,
            PlayerFieldPositionGroup::Midfielder => 2,
            PlayerFieldPositionGroup::Forward => 3,
        }
    }

    pub fn as_i18n_key(&self) -> &'static str {
        match self {
            PlayerFieldPositionGroup::Goalkeeper => "pos_group_goalkeeper",
            PlayerFieldPositionGroup::Defender => "pos_group_defender",
            PlayerFieldPositionGroup::Midfielder => "pos_group_midfielder",
            PlayerFieldPositionGroup::Forward => "pos_group_forward",
        }
    }

    /// Baseline "ideal" senior squad depth for this position group, used by
    /// the recruitment, shortlist and loan-market depth checks. Centralizes
    /// the GK 3 / DEF 8 / MID 8 / FWD 6 table that was duplicated across
    /// three pipeline modules so it can no longer drift between them. Tier
    /// and fixture-load scaling layer on top of this baseline elsewhere.
    pub fn ideal_squad_depth(&self) -> usize {
        match self {
            PlayerFieldPositionGroup::Goalkeeper => 3,
            PlayerFieldPositionGroup::Defender => 8,
            PlayerFieldPositionGroup::Midfielder => 8,
            PlayerFieldPositionGroup::Forward => 6,
        }
    }

    /// Main-team depth cap per group enforced by the weekly squad rebalance:
    /// players ranked beyond it are demoted/loan-listed as positional
    /// surplus. Slightly looser than [`Self::ideal_squad_depth`] — the cap
    /// is where the club starts shedding bodies, not the roster it aims
    /// for. The transfer pipeline's squad-fit gate reads the same cap so a
    /// club never buys a player its own rebalance would immediately demote.
    ///
    /// The `Defender` gap over the ideal is deliberately the widest: that
    /// group covers two lines of the pitch, since `DefensiveMidfielder`
    /// maps into it (see [`Self::typical_starters`]). At the old cap of 9 a
    /// club carrying an ordinary eight defenders plus four holding
    /// midfielders was permanently three bodies "over", and the rebalance
    /// answered by demoting and loan-listing genuine first-team defenders
    /// every week.
    pub fn main_depth_cap(&self) -> usize {
        match self {
            PlayerFieldPositionGroup::Goalkeeper => 3,
            PlayerFieldPositionGroup::Defender => 12,
            PlayerFieldPositionGroup::Midfielder => 9,
            PlayerFieldPositionGroup::Forward => 6,
        }
    }

    /// Fewest players of this group a senior squad can function with — below
    /// it the club cannot field a balanced matchday side and the position
    /// reads as a genuine need. The counterpart to [`Self::main_depth_cap`],
    /// which is where a squad starts reading as over-stocked.
    pub fn minimum_viable_depth(&self) -> usize {
        match self {
            PlayerFieldPositionGroup::Goalkeeper => 2,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }

    /// Does a squad carrying `count` players of this group have more than it
    /// can use? The one shared answer for every "is this position surplus?"
    /// question, so the listing sweep, the weekly rebalance and the buy-side
    /// squad-fit gate can never disagree about it.
    pub fn is_over_stocked(&self, count: usize) -> bool {
        count > self.main_depth_cap()
    }

    /// Does a squad carrying `count` players of this group need another?
    pub fn is_under_stocked(&self, count: usize) -> bool {
        count < self.minimum_viable_depth()
    }

    /// How many of this group start a typical match — the number of
    /// "regular" slots the position offers. A keeper has exactly one, which
    /// is why a keeper's number two is a backup rather than a rotation
    /// regular. Used by [`crate::PlayerSquadStatus::calculate`] to judge a
    /// player's role against the slots available at his position instead of
    /// a flat percentile of the group.
    ///
    /// The figures must track what each group actually *contains* (see
    /// [`PlayerPositionType::position_group`]), not the shirt names: the
    /// `Defender` group holds the back four **plus** the holding midfield
    /// slots, because `DefensiveMidfielder` maps into it. Counting it as
    /// four starters ranked a top-flight centre-back behind his club's
    /// holding midfielders and relabelled last season's starter as rotation
    /// depth — which the disposal sweeps then read as loanable.
    pub fn typical_starters(&self) -> usize {
        match self {
            PlayerFieldPositionGroup::Goalkeeper => 1,
            // Back four + the one or two holding slots that map here.
            PlayerFieldPositionGroup::Defender => 6,
            // Wide and central midfield plus the attacking-midfield and
            // wingback slots that map here.
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }
}

#[cfg(test)]
mod role_capability_tests {
    use super::*;

    struct RoleFx;

    impl RoleFx {
        fn positions(entries: &[(PlayerPositionType, u8)]) -> PlayerPositions {
            PlayerPositions {
                positions: entries
                    .iter()
                    .map(|(position, level)| PlayerPosition {
                        position: *position,
                        level: *level,
                    })
                    .collect(),
            }
        }

        /// The shape that started all this: a forward whose record happens to
        /// list a wing first, so his one primary label reads Midfielder while
        /// he leads the line at full competence.
        fn wide_forward() -> PlayerPositions {
            Self::positions(&[
                (PlayerPositionType::AttackingMidfielderRight, 20),
                (PlayerPositionType::AttackingMidfielderLeft, 20),
                (PlayerPositionType::Striker, 20),
            ])
        }
    }

    #[test]
    fn coverage_sees_every_role_the_label_hides() {
        let player = RoleFx::wide_forward();
        assert_eq!(
            player.primary().unwrap().position_group(),
            PlayerFieldPositionGroup::Midfielder,
            "the single primary label is the first listed role — a tie-break, not a verdict"
        );

        let coverage = PositionCoverage::of(&player);
        assert!(coverage.covers(PlayerPositionType::Striker));
        assert!(
            coverage.covers_group(PlayerFieldPositionGroup::Forward),
            "a search for a centre-forward has to be able to find him"
        );
        assert!(coverage.covers_group(PlayerFieldPositionGroup::Midfielder));
        assert!(!coverage.covers_group(PlayerFieldPositionGroup::Defender));
    }

    #[test]
    fn coverage_ignores_roles_below_playable_familiarity() {
        let dabbler = RoleFx::positions(&[
            (PlayerPositionType::DefenderCenter, 20),
            (PlayerPositionType::Striker, 2),
        ]);
        let coverage = PositionCoverage::of(&dabbler);
        assert!(!coverage.covers(PlayerPositionType::Striker));
        assert!(!coverage.covers_group(PlayerFieldPositionGroup::Forward));
    }

    #[test]
    fn coverage_falls_back_to_the_best_listed_role() {
        // Mirrors `PlayerPositions::positions`: a player whose every entry
        // sits under the bar still has to be somewhere.
        let raw = RoleFx::positions(&[
            (PlayerPositionType::Goalkeeper, 3),
            (PlayerPositionType::DefenderCenter, 4),
        ]);
        let coverage = PositionCoverage::of(&raw);
        assert!(coverage.covers(PlayerPositionType::DefenderCenter));
        assert!(!coverage.is_empty());
    }

    #[test]
    fn group_masks_partition_the_position_space() {
        let mut seen = 0u32;
        for group in PlayerFieldPositionGroup::ALL {
            let mask = group.coverage_mask();
            assert_eq!(mask & seen, 0, "{group:?} overlaps an earlier group");
            seen |= mask;
        }
        for position in PlayerPositionType::ALL {
            assert!(
                PositionCoverage::single(position).covers_group(position.position_group()),
                "{position:?} must fall inside its own group's mask"
            );
        }
    }

    #[test]
    fn familiarity_credit_is_continuous_and_monotone() {
        let mut previous = 0.0_f32;
        for level in 0..=20u8 {
            let credit = RoleFamiliarity::credit(level);
            assert!(
                credit >= previous - f32::EPSILON,
                "credit must never fall as familiarity rises (level {level})"
            );
            // No band boundary may move a player by more than a nudge.
            assert!(
                credit - previous < 0.51,
                "level {level} is a cliff, not a curve"
            );
            previous = credit;
        }
        assert_eq!(RoleFamiliarity::credit(20), 1.0);
        assert_eq!(RoleFamiliarity::credit(18), 1.0);
        assert_eq!(RoleFamiliarity::credit(4), 0.0, "below playable is nothing");
    }

    #[test]
    fn a_nominal_second_position_does_not_read_as_a_real_one() {
        // The gap that matters: a winger who lists centre-forward at eight
        // must not be mistaken for a centre-forward, or he papers over the
        // very hole the club needs to fill.
        let nominal = RoleFamiliarity::effective_ability(150, 8);
        let natural = RoleFamiliarity::effective_ability(150, 20);
        assert!(
            nominal < natural,
            "an unconvincing familiarity has to cost something"
        );
        assert!(
            natural - nominal >= 30,
            "…and enough to matter against a tier baseline"
        );
    }

    #[test]
    fn best_in_group_reads_the_strongest_role_the_player_holds_there() {
        let player = RoleFx::wide_forward();
        assert_eq!(
            RoleFamiliarity::best_in_group(&player, 150, PlayerFieldPositionGroup::Forward),
            150,
            "a natural centre-forward counts fully as one, whatever he is filed under"
        );
        assert_eq!(
            RoleFamiliarity::best_in_group(&player, 150, PlayerFieldPositionGroup::Defender),
            0,
            "and not at all where he cannot play"
        );
    }
}
