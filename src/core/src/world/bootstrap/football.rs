//! Every player of the opening world starts with the career he has had:
//! the football he has banked, from the seasons he arrives with or from his
//! place in the side, then his timing and his blueprint.

use crate::Player;
use crate::PlayerFieldPositionGroup;
use crate::club::player::development::SquadRole;
use crate::world::SimulatorData;
use rayon::prelude::*;
use rustc_hash::FxHashSet;

impl SimulatorData {
    /// Bank every rostered player's football once, at world construction:
    /// from every season of his record where he has one, else from his
    /// ability rank in his position group. A player created during play
    /// starts empty.
    pub(in crate::world) fn seed_player_football(&mut self) {
        let opening = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let friendly_leagues: FxHashSet<u32> = country
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
                        let role = group.map(|group| {
                            let rank = places
                                .iter()
                                .filter(|&&(g, c)| g == Some(group) && c > ca)
                                .count();
                            SquadRole::of(rank, group.typical_starters(), plays_friendlies)
                        });
                        player.seed_football(opening, role);
                    }
                }
            });
    }

    /// The free agents a world opens with, given the football and the
    /// development their records imply. They join the pool after the clubs
    /// are seeded, so the club passes never see them.
    pub fn admit_opening_free_agents(&mut self, players: Vec<Player>) {
        let opening = self.date.date();
        for mut player in players {
            player.seed_football(opening, None);
            player.seed_development(opening);
            self.free_agents.push(player);
        }
    }
}

impl SimulatorData {
    /// Give every player of the opening world the development state his
    /// record implies: rostered players and academy boys.
    pub(in crate::world) fn seed_player_development(&mut self) {
        let opening = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                for club in &mut country.clubs {
                    for player in club
                        .teams
                        .iter_mut()
                        .flat_map(|t| t.players.players.iter_mut())
                    {
                        player.seed_development(opening);
                    }
                    for player in club.academy.players.players.iter_mut() {
                        player.seed_development(opening);
                    }
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use crate::club::player::builder::PlayerBuilder;
    use crate::club::player::development::SquadRole;
    use crate::club::player::maturation::MaturationGroup;
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

    fn banked(p: &Player) -> f32 {
        p.development.football.share()
    }

    #[test]
    fn a_regular_on_the_worlds_first_day() {
        let mut p = player(PlayerPositionType::MidfielderCenter);
        p.birth_date = NaiveDate::from_ymd_opt(2001, 3, 1).unwrap();
        for year in 2021..=2025 {
            p.statistics_history.items.push(season(year, 30));
        }
        p.seed_football(opening(), None);
        for family in MaturationGroup::ALL {
            if family == MaturationGroup::Goalkeeping {
                continue;
            }
            let share = p.development.football.of(family);
            assert!(share >= 0.999, "{family:?} banked {share:.3}");
        }
    }

    #[test]
    fn a_third_keeper_who_has_never_played() {
        let mut keeper = player(PlayerPositionType::Goalkeeper);
        keeper.birth_date = NaiveDate::from_ymd_opt(2002, 3, 1).unwrap();
        keeper.seed_football(
            opening(),
            Some(SquadRole::of(
                2,
                PlayerFieldPositionGroup::Goalkeeper.typical_starters(),
                false,
            )),
        );
        assert!(
            banked(&keeper) < 0.2,
            "third-choice keeper banked {:.3}",
            banked(&keeper)
        );
    }

    #[test]
    fn a_season_split_across_two_clubs_counts_whole() {
        let mut whole = player(PlayerPositionType::Striker);
        whole.statistics_history.items.push(season(2025, 20));
        let mut split = player(PlayerPositionType::Striker);
        split.statistics_history.items.push(season(2025, 12));
        split.statistics_history.items.push(season(2025, 8));
        whole.seed_football(opening(), None);
        split.seed_football(opening(), None);
        assert!((banked(&whole) - banked(&split)).abs() < 1e-4);
    }

    #[test]
    fn a_youth_league_regular_banks_half_a_regulars_football() {
        let mut senior = player(PlayerPositionType::Striker);
        let mut youth = player(PlayerPositionType::Striker);
        senior.birth_date = NaiveDate::from_ymd_opt(2008, 3, 1).unwrap();
        youth.birth_date = senior.birth_date;
        senior.seed_football(opening(), Some(SquadRole::of(0, 2, false)));
        youth.seed_football(opening(), Some(SquadRole::of(0, 2, true)));
        assert!((banked(&youth) - banked(&senior) * 0.5).abs() < 0.01);
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
        assert_eq!(banked(&intake), 0.0);
    }
}
