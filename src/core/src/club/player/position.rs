use serde::Serialize;
use std::fmt::{Display, Formatter, Result};

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, PartialOrd, Serialize)]
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

    #[inline]
    pub fn position_group(&self) -> PlayerFieldPositionGroup {
        match *self {
            PlayerPositionType::Goalkeeper => PlayerFieldPositionGroup::Goalkeeper,
            PlayerPositionType::Sweeper |
            PlayerPositionType::DefenderLeft |
            PlayerPositionType::DefenderCenterLeft |
            PlayerPositionType::DefenderCenter |
            PlayerPositionType::DefenderCenterRight |
            PlayerPositionType::DefenderRight |
            PlayerPositionType::DefensiveMidfielder => PlayerFieldPositionGroup::Defender,
            PlayerPositionType::MidfielderLeft |
            PlayerPositionType::MidfielderCenterLeft |
            PlayerPositionType::MidfielderCenter |
            PlayerPositionType::MidfielderCenterRight |
            PlayerPositionType::MidfielderRight |
            PlayerPositionType::AttackingMidfielderLeft |
            PlayerPositionType::AttackingMidfielderCenter |
            PlayerPositionType::AttackingMidfielderRight |
            PlayerPositionType::WingbackLeft |
            PlayerPositionType::WingbackRight => PlayerFieldPositionGroup::Midfielder,
            PlayerPositionType::ForwardLeft |
            PlayerPositionType::ForwardCenter |
            PlayerPositionType::ForwardRight |
            PlayerPositionType::Striker => PlayerFieldPositionGroup::Forward,
        }
    }
}

#[derive(Debug)]
pub struct PlayerPositions {
    pub positions: Vec<PlayerPosition>,
}

const REQUIRED_POSITION_LEVEL: u8 = 15;

impl PlayerPositions {
    pub fn positions(&self) -> Vec<PlayerPositionType> {
        self.positions
            .iter()
            .filter(|p| p.level >= REQUIRED_POSITION_LEVEL)
            .map(|p| p.position)
            .collect()
    }

    pub fn display_positions(&self) -> Vec<&str> {
        self.positions()
            .iter()
            .map(|p| p.get_short_name())
            .collect()
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

#[derive(Debug)]
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

        assert_eq!("WL,WR", display_positions);
    }
}

#[derive(PartialEq, Eq, Hash, Debug, Clone, Copy)]
pub enum PlayerFieldPositionGroup {
    Goalkeeper,
    Defender,
    Midfielder,
    Forward,
}
