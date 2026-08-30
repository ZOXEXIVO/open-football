use crate::PlayerPositionType;

/// The pitch length these coordinates are written against. Both goals
/// sit on it (x = 0 and x = `PITCH_LENGTH`), which is what makes the
/// away column a reflection of the home one.
pub const PITCH_LENGTH: i16 = 840;

/// Where every role lines up at a restart, for each end of the pitch.
///
/// # The away column is the home column, reflected
///
/// `away_x == PITCH_LENGTH - home_x`, and `away_y` is the home `y` of
/// the role's mirror (away `DefenderLeft` stands on 450 because home
/// `DefenderRight` does). Nothing re-mirrors these downstream —
/// `get_player_position` and `TacticalPositions` read them as literal
/// pitch coordinates — so any asymmetry here is a permanent handicap to
/// one side, applied at every kickoff, every goal and every half.
///
/// ⚠ **It used to be asymmetric on both axes, and one of them was worth
/// goals.** Measured against a proper reflection, the away side lined up
/// systematically DEEPER: forwards **35 u (4.4 m)** further from the goal
/// they were attacking, defenders / defensive midfielder / wingbacks
/// 20 u, midfielders 17 u, attacking midfielders 5 u, with only the
/// goalkeeper and the striker exact. `start_position` is a fallback
/// anchor and a base for several states' off-ball targets, so it is
/// load-bearing for the whole match rather than the first ten seconds —
/// an undeclared second home-advantage channel, in an engine whose
/// documented position is that `crowd_arousal` is the only one, and
/// which was already returning roughly twice the real home edge.
///
/// (The other axis: both wingback rows carried their home `y` across
/// unchanged, so the away side of a 3-5-2 or a 3-4-3 put the man picked
/// for its left touchline on the right one — and the route drawn for
/// him, which mirrors correctly, then ran him diagonally across the
/// whole pitch.)
///
/// `away_positions_mirror_the_home_ones` holds both axes.
pub const POSITION_POSITIONING: &[(PlayerPositionType, PositionType, PositionType)] = &[
    (
        PlayerPositionType::Goalkeeper,
        PositionType::Home(20, 275),
        PositionType::Away(820, 275),
    ),
    (
        PlayerPositionType::Sweeper,
        PositionType::Home(80, 275),
        PositionType::Away(760, 275),
    ),
    (
        PlayerPositionType::DefenderLeft,
        PositionType::Home(165, 85),
        PositionType::Away(675, 450),
    ),
    (
        PlayerPositionType::DefenderCenterLeft,
        PositionType::Home(165, 210),
        PositionType::Away(675, 330),
    ),
    (
        PlayerPositionType::DefenderCenter,
        PositionType::Home(165, 275),
        PositionType::Away(675, 275),
    ),
    (
        PlayerPositionType::DefenderCenterRight,
        PositionType::Home(165, 330),
        PositionType::Away(675, 210),
    ),
    (
        PlayerPositionType::DefenderRight,
        PositionType::Home(165, 450),
        PositionType::Away(675, 85),
    ),
    (
        PlayerPositionType::DefensiveMidfielder,
        PositionType::Home(230, 275),
        PositionType::Away(610, 275),
    ),
    // ⚠ These two rows are the ones that used to keep their home `y`
    // instead of the mirror's — see the width paragraph on the const.
    (
        PlayerPositionType::WingbackLeft,
        PositionType::Home(235, 50),
        PositionType::Away(605, 480),
    ),
    (
        PlayerPositionType::WingbackRight,
        PositionType::Home(235, 480),
        PositionType::Away(605, 50),
    ),
    (
        PlayerPositionType::MidfielderLeft,
        PositionType::Home(297, 85),
        PositionType::Away(543, 450),
    ),
    (
        PlayerPositionType::MidfielderCenterLeft,
        PositionType::Home(297, 210),
        PositionType::Away(543, 330),
    ),
    (
        PlayerPositionType::MidfielderCenter,
        PositionType::Home(297, 275),
        PositionType::Away(543, 275),
    ),
    (
        PlayerPositionType::MidfielderCenterRight,
        PositionType::Home(297, 330),
        PositionType::Away(543, 210),
    ),
    (
        PlayerPositionType::MidfielderRight,
        PositionType::Home(297, 450),
        PositionType::Away(543, 85),
    ),
    (
        PlayerPositionType::AttackingMidfielderLeft,
        PositionType::Home(360, 150),
        PositionType::Away(480, 385),
    ),
    (
        PlayerPositionType::AttackingMidfielderCenter,
        PositionType::Home(360, 275),
        PositionType::Away(480, 275),
    ),
    (
        PlayerPositionType::AttackingMidfielderRight,
        PositionType::Home(360, 385),
        PositionType::Away(480, 150),
    ),
    (
        PlayerPositionType::ForwardLeft,
        PositionType::Home(395, 210),
        PositionType::Away(445, 330),
    ),
    (
        PlayerPositionType::ForwardCenter,
        PositionType::Home(395, 275),
        PositionType::Away(445, 275),
    ),
    (
        PlayerPositionType::ForwardRight,
        PositionType::Home(395, 330),
        PositionType::Away(445, 210),
    ),
    (
        PlayerPositionType::Striker,
        PositionType::Home(405, 275),
        PositionType::Away(435, 275),
    ),
];

pub enum PositionType {
    Home(i16, i16),
    Away(i16, i16),
}

#[cfg(test)]
mod tests {
    use super::{PITCH_LENGTH, POSITION_POSITIONING, PositionType};
    use crate::PlayerPositionType;

    struct PitchMirror;

    impl PitchMirror {
        /// The slot on the other side of the pitch from this one. A central
        /// slot mirrors onto itself.
        fn of(position: PlayerPositionType) -> PlayerPositionType {
            match position {
                PlayerPositionType::DefenderLeft => PlayerPositionType::DefenderRight,
                PlayerPositionType::DefenderRight => PlayerPositionType::DefenderLeft,
                PlayerPositionType::DefenderCenterLeft => PlayerPositionType::DefenderCenterRight,
                PlayerPositionType::DefenderCenterRight => PlayerPositionType::DefenderCenterLeft,
                PlayerPositionType::WingbackLeft => PlayerPositionType::WingbackRight,
                PlayerPositionType::WingbackRight => PlayerPositionType::WingbackLeft,
                PlayerPositionType::MidfielderLeft => PlayerPositionType::MidfielderRight,
                PlayerPositionType::MidfielderRight => PlayerPositionType::MidfielderLeft,
                PlayerPositionType::MidfielderCenterLeft => {
                    PlayerPositionType::MidfielderCenterRight
                }
                PlayerPositionType::MidfielderCenterRight => {
                    PlayerPositionType::MidfielderCenterLeft
                }
                PlayerPositionType::AttackingMidfielderLeft => {
                    PlayerPositionType::AttackingMidfielderRight
                }
                PlayerPositionType::AttackingMidfielderRight => {
                    PlayerPositionType::AttackingMidfielderLeft
                }
                PlayerPositionType::ForwardLeft => PlayerPositionType::ForwardRight,
                PlayerPositionType::ForwardRight => PlayerPositionType::ForwardLeft,
                other => other,
            }
        }

        fn home_of(position: PlayerPositionType) -> (i16, i16) {
            for (pos, home, _) in POSITION_POSITIONING {
                if *pos == position {
                    if let PositionType::Home(x, y) = home {
                        return (*x, *y);
                    }
                }
            }
            panic!("{position:?} has no home row");
        }
    }

    /// **Both ends of the pitch line up the same way round.**
    ///
    /// The away column is the home column reflected in the halfway line:
    /// `away_x == PITCH_LENGTH - home_x`, and the away lateral
    /// coordinate is the home lateral coordinate of the role's MIRROR —
    /// away `DefenderLeft` stands on 450 because home `DefenderRight`
    /// does.
    ///
    /// Both axes used to be broken, and both mattered.
    ///
    /// **Depth**: the away side lined up systematically deeper — forwards
    /// 35 u (4.4 m), defenders / DM / wingbacks 20 u, midfielders 17 u,
    /// attacking midfielders 5 u; only the goalkeeper and the striker
    /// were exact. Nothing re-mirrors this table downstream, and
    /// `start_position` is a fallback anchor rather than a kickoff-only
    /// value, so that was a permanent territorial handicap to whoever was
    /// away — an undeclared home-advantage channel beside the titrated
    /// one.
    ///
    /// **Width**: both wingback rows carried their home `y` across
    /// unchanged, putting the away side of a 3-5-2 or a 3-4-3 with its
    /// left wingback on the right touchline — and the route drawn for
    /// him, which mirrors correctly, then ran him diagonally across the
    /// whole pitch. Nothing else in the table did it, and nothing
    /// checked.
    #[test]
    fn away_positions_mirror_the_home_ones() {
        for (position, _, away) in POSITION_POSITIONING {
            let PositionType::Away(away_x, away_y) = away else {
                panic!("{position:?} away row is not an Away variant");
            };
            let (own_home_x, _) = PitchMirror::home_of(*position);
            assert_eq!(
                *away_x,
                PITCH_LENGTH - own_home_x,
                "{position:?}: away x {away_x} should be the reflection of home x {own_home_x}"
            );

            let mirror = PitchMirror::of(*position);
            let (_, mirror_home_y) = PitchMirror::home_of(mirror);
            assert_eq!(
                *away_y, mirror_home_y,
                "{position:?}: away y {away_y} should mirror home {mirror:?} y {mirror_home_y}"
            );
        }
    }

    /// Every slot a formation can ask for has a row here, or it silently
    /// lines up on the centre spot (`get_base_position_coordinates`
    /// falls back to the middle of the pitch).
    #[test]
    fn every_position_has_a_row() {
        for position in PlayerPositionType::ALL {
            assert!(
                POSITION_POSITIONING.iter().any(|(p, _, _)| p == &position),
                "{position:?} has no row in POSITION_POSITIONING"
            );
        }
    }
}
