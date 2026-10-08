//! Every player's football absorbed — the minutes that open the
//! match-learned share of his ceilings — set from the seasons he arrives
//! with, or from his place in the side he plays for.

use crate::PlayerFieldPositionGroup;
use crate::world::SimulatorData;
use rayon::prelude::*;
use std::collections::HashSet;

impl SimulatorData {
    /// Seed every rostered player's football absorbed once, at world
    /// construction: from his two most recent recorded seasons where he has
    /// them, else from his ability rank in his position group. Free agents
    /// are seeded from a recorded career only; a player created during play
    /// starts empty.
    pub(in crate::world) fn seed_player_football(&mut self) {
        let opening = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let friendly_leagues: HashSet<u32> = country
                    .leagues
                    .leagues
                    .iter()
                    .filter(|league| league.friendly)
                    .map(|league| league.id)
                    .collect();
                for team in country.clubs.iter_mut().flat_map(|c| c.teams.iter_mut()) {
                    let plays_friendlies = team
                        .league_id
                        .is_none_or(|id| friendly_leagues.contains(&id));
                    let places: Vec<(Option<PlayerFieldPositionGroup>, u8)> = team
                        .players
                        .iter()
                        .map(|p| {
                            let group = p.positions.primary().map(|pos| pos.position_group());
                            (group, p.player_attributes.current_ability)
                        })
                        .collect();
                    for (player, &(group, ca)) in team.players.players.iter_mut().zip(&places) {
                        if player.seed_football_from_career(opening) {
                            continue;
                        }
                        let Some(group) = group else { continue };
                        let rank = places
                            .iter()
                            .filter(|&&(g, c)| g == Some(group) && c > ca)
                            .count();
                        player.seed_football_from_role(group, rank, plays_friendlies);
                    }
                }
            });
        for player in &mut self.free_agents {
            player.seed_football_from_career(opening);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::statistics::{PlayerStatistics, PlayerStatisticsHistoryItem};
    use crate::league::Season;
    use crate::shared::fullname::FullName;
    use crate::{
        AcademyGenerationContext, PeopleNameGeneratorData, PersonAttributes, Player,
        PlayerAttributes, PlayerFieldPositionGroup, PlayerGenerator, PlayerPosition,
        PlayerPositionType, PlayerPositions, PlayerSkills,
    };
    use chrono::NaiveDate;

    fn opening() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 1).unwrap()
    }

    fn player(position: PlayerPositionType) -> Player {
        PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Test".to_string(), "Player".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(1998, 3, 1).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position,
                    level: 20,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap()
    }

    fn season(year: u16, starts: u16) -> PlayerStatisticsHistoryItem {
        PlayerStatisticsHistoryItem {
            season: Season::new(year),
            team_name: "club".to_string(),
            team_slug: "club".to_string(),
            team_reputation: 3000,
            league_name: "league".to_string(),
            league_slug: "league".to_string(),
            is_loan: false,
            transfer_fee: None,
            statistics: PlayerStatistics {
                played: starts,
                ..PlayerStatistics::default()
            },
            seq_id: year as u32,
        }
    }

    #[test]
    fn a_regular_of_his_last_two_seasons_opens_the_world_fully_exposed() {
        let mut p = player(PlayerPositionType::MidfielderCenter);
        p.statistics_history.items.push(season(2019, 0));
        p.statistics_history.items.push(season(2024, 30));
        p.statistics_history.items.push(season(2025, 30));
        assert!(p.seed_football_from_career(opening()));
        assert_eq!(p.load.match_exposure(), 1.0);
    }

    #[test]
    fn a_season_split_across_two_clubs_counts_whole() {
        let mut whole = player(PlayerPositionType::Striker);
        whole.statistics_history.items.push(season(2025, 20));
        let mut split = player(PlayerPositionType::Striker);
        split.statistics_history.items.push(season(2025, 12));
        split.statistics_history.items.push(season(2025, 8));
        whole.seed_football_from_career(opening());
        split.seed_football_from_career(opening());
        assert!((whole.load.football_absorbed - split.load.football_absorbed).abs() < 1e-3);
    }

    #[test]
    fn a_player_with_no_record_is_seeded_by_his_place_in_the_side() {
        let mut third_keeper = player(PlayerPositionType::Goalkeeper);
        assert!(!third_keeper.seed_football_from_career(opening()));
        third_keeper.seed_football_from_role(PlayerFieldPositionGroup::Goalkeeper, 2, false);
        let exposure = third_keeper.load.match_exposure();
        assert!(
            (exposure - 0.1).abs() < 0.01,
            "third-choice keeper reads {exposure}"
        );

        let mut youth_regular = player(PlayerPositionType::Striker);
        youth_regular.seed_football_from_role(PlayerFieldPositionGroup::Forward, 0, true);
        assert!((youth_regular.load.match_exposure() - 0.5).abs() < 0.01);
    }

    #[test]
    fn an_academy_intake_starts_with_no_football() {
        let names = PeopleNameGeneratorData {
            first_names: Vec::new(),
            last_names: Vec::new(),
            nicknames: Vec::new(),
        };
        let ctx =
            AcademyGenerationContext::from_components(12, 0.6, 0.6, 0.6, 0.6, 5000, 5000, 5000, 60);
        let intake = PlayerGenerator::generate_with_context(
            1,
            "",
            opening(),
            PlayerPositionType::Goalkeeper,
            &names,
            &ctx,
            15,
            15,
            None,
        );
        assert_eq!(intake.load.match_exposure(), 0.0);
    }
}
