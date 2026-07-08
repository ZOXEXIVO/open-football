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

    pub fn has_position(&self, position: PlayerPositionType) -> bool {
        self.positions().contains(&position)
    }

    pub fn is_goalkeeper(&self) -> bool {
        self.positions().contains(&PlayerPositionType::Goalkeeper)
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

    /// How many of this group start a typical match — the number of
    /// "regular" slots the position offers. A keeper has exactly one, which
    /// is why a keeper's number two is a backup rather than a rotation
    /// regular; the outfield figures track a conventional 4-4-2 shape.
    /// Used by [`crate::PlayerSquadStatus::calculate`] to judge a player's
    /// role against the slots available at his position instead of a flat
    /// percentile of the group.
    pub fn typical_starters(&self) -> usize {
        match self {
            PlayerFieldPositionGroup::Goalkeeper => 1,
            PlayerFieldPositionGroup::Defender => 4,
            PlayerFieldPositionGroup::Midfielder => 4,
            PlayerFieldPositionGroup::Forward => 2,
        }
    }
}
