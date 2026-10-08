//! The standard of football a side plays at, read off its strongest
//! eleven through the same composites a match reads at kickoff — so a
//! player's assurance, learnt from matches, and a team's standard, read
//! from its squad, live on one scale.

use crate::r#match::MatchPlayer;
use crate::r#match::engine::teamplay::standard::MatchStandard;
use crate::r#match::engine::teamplay::tactical::TeamSkillAggregates;
use crate::{CONDITION_MAX_VALUE, Player, PlayerPositionType};
use std::cmp::Reverse;

pub struct PlayingStandard;

impl PlayingStandard {
    const OUTFIELDERS: usize = 10;

    /// The standard `players` would play at as one side: the best keeper
    /// and the ten strongest outfielders, fresh. The calibration standard
    /// for a squad with no outfielder in it.
    pub fn of(players: &[Player]) -> f32 {
        let placed: Vec<(&Player, PlayerPositionType)> = players
            .iter()
            .filter_map(|p| p.positions.primary().map(|position| (p, position)))
            .collect();
        let mut outfield: Vec<(&Player, PlayerPositionType)> = placed
            .iter()
            .copied()
            .filter(|(_, position)| !position.is_goalkeeper())
            .collect();
        if outfield.is_empty() {
            return MatchStandard::CALIBRATION;
        }
        outfield.sort_by_key(|(p, _)| Reverse(p.player_attributes.current_ability));
        let keeper = placed
            .iter()
            .copied()
            .filter(|(_, position)| position.is_goalkeeper())
            .max_by_key(|(p, _)| p.player_attributes.current_ability);
        let eleven: Vec<MatchPlayer> = keeper
            .into_iter()
            .chain(outfield.into_iter().take(Self::OUTFIELDERS))
            .map(|(p, position)| {
                let mut mp = MatchPlayer::from_player(0, p, position, false, None);
                mp.player_attributes.condition = CONDITION_MAX_VALUE;
                mp
            })
            .collect();
        MatchStandard::of_side(&TeamSkillAggregates::at_kickoff(&eleven))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::builder::PlayerBuilder;
    use crate::shared::fullname::FullName;
    use crate::{
        PersonAttributes, PlayerAttributes, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills,
    };
    use chrono::NaiveDate;

    const SHAPE: [PlayerPositionType; 11] = [
        PlayerPositionType::Goalkeeper,
        PlayerPositionType::DefenderLeft,
        PlayerPositionType::DefenderCenterLeft,
        PlayerPositionType::DefenderCenterRight,
        PlayerPositionType::DefenderRight,
        PlayerPositionType::MidfielderLeft,
        PlayerPositionType::MidfielderCenterLeft,
        PlayerPositionType::MidfielderCenterRight,
        PlayerPositionType::MidfielderRight,
        PlayerPositionType::ForwardLeft,
        PlayerPositionType::ForwardRight,
    ];

    fn squad(ability: u8) -> Vec<Player> {
        SHAPE
            .iter()
            .enumerate()
            .map(|(i, &position)| {
                PlayerBuilder::new()
                    .id(i as u32 + 1)
                    .full_name(FullName::new("T".to_string(), format!("P{i}")))
                    .birth_date(NaiveDate::from_ymd_opt(1998, 1, 1).unwrap())
                    .country_id(1)
                    .attributes(PersonAttributes::default())
                    .skills(PlayerSkills::flat_for_ability(ability))
                    .positions(PlayerPositions {
                        positions: vec![PlayerPosition { position, level: 20 }],
                    })
                    .player_attributes(PlayerAttributes {
                        current_ability: ability,
                        condition: CONDITION_MAX_VALUE,
                        ..PlayerAttributes::default()
                    })
                    .build()
                    .unwrap()
            })
            .collect()
    }

    #[test]
    fn a_stronger_squad_plays_at_a_higher_standard() {
        assert!(PlayingStandard::of(&squad(140)) > PlayingStandard::of(&squad(80)));
    }

    #[test]
    fn a_squad_with_no_outfielders_reads_the_calibration_standard() {
        let keeper_only: Vec<Player> = squad(120).into_iter().take(1).collect();
        assert_eq!(PlayingStandard::of(&keeper_only), MatchStandard::CALIBRATION);
    }
}
