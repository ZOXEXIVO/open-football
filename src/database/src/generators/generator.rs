use crate::generators::{PlayerGenerator, PositionType, StaffGenerator};
use crate::loaders::ContinentEntity;
use crate::DatabaseEntity;
use chrono::{NaiveDate, NaiveDateTime};
use core::club::academy::ClubAcademy;
use core::context::NaiveTime;
use core::continent::Continent;
use core::league::LeagueCollection;
use core::league::Schedule;
use core::league::{DayMonthPeriod, League, LeagueSettings, LeagueTable};
use core::shared::Location;
use core::utils::IntegerUtils;
use core::ClubStatus;
use core::TeamCollection;
use core::{
    Club, ClubBoard, ClubFinances, ClubMood, Country, CountryGeneratorData, Player,
    PlayerCollection, SimulatorData, Staff, StaffCollection, StaffPosition, Team,
    TeamReputation, TeamType, TrainingSchedule,
};
use std::str::FromStr;
use core::league::MatchStorage;

pub struct DatabaseGenerator;

impl DatabaseGenerator {
    pub fn generate(data: &DatabaseEntity) -> SimulatorData {
        let current_date = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2024, 7, 1).unwrap(),
            NaiveTime::default(),
        );

        let continents = data
            .continents
            .iter()
            .map(|continent| Continent::new(                
                continent.id,
                continent.name.clone(),
                DatabaseGenerator::generate_countries(continent, data)
            )).collect();

        SimulatorData::new(current_date, continents)
    }

    fn generate_countries(continent: &ContinentEntity, data: &DatabaseEntity) -> Vec<Country> {
        data
            .countries
            .iter()
            .filter(|cn| cn.continent_id == continent.id)
            .map(|country| {
                let generator_data = match data
                    .names_by_country
                    .iter()
                    .find(|c| c.country_id == country.id)
                {
                    Some(names) => CountryGeneratorData::new(
                        names.first_names.clone(),
                        names.last_names.clone(),
                    ),
                    None => CountryGeneratorData::empty(),
                };

                let mut player_generator =
                    PlayerGenerator::with_people_names(&generator_data.people_names);

                let mut staff_generator =
                    StaffGenerator::with_people_names(&generator_data.people_names);

                let clubs = DatabaseGenerator::generate_clubs(
                    country.id,
                    data,
                    &mut player_generator,
                    &mut staff_generator,
                );

                let country = Country::new(
                    country.id,
                    country.code.clone(),
                    country.slug.clone(),
                    country.name.clone(),
                    continent.id,
                    LeagueCollection::new(DatabaseGenerator::generate_leagues(
                        country.id, data,
                    )),
                    clubs,
                    country.reputation,
                    generator_data
                );

                country
            })
            .collect()
    }

    fn generate_leagues(country_id: u32, data: &DatabaseEntity) -> Vec<League> {
        data
            .leagues
            .iter()
            .filter(|l| l.country_id == country_id)
            .map(|league| {
                let league_clubs: Vec<u32> = data
                    .clubs
                    .iter()
                    .flat_map(|c| &c.teams)
                    .filter(|team| team.league_id == league.id)
                    .map(|t| t.id)
                    .collect();

                let settings = LeagueSettings {
                    season_starting_half: DayMonthPeriod {
                        from_day: league.settings.season_starting_half.from_day,
                        from_month: league.settings.season_starting_half.from_month,
                        to_day: league.settings.season_starting_half.to_day,
                        to_month: league.settings.season_starting_half.to_month,
                    },
                    season_ending_half: DayMonthPeriod {
                        from_day: league.settings.season_ending_half.from_day,
                        from_month: league.settings.season_ending_half.from_month,
                        to_day: league.settings.season_ending_half.to_day,
                        to_month: league.settings.season_ending_half.to_month,
                    },
                };
                
                League::new(league.id, league.name.clone(), league.slug.clone(), league.country_id, 0, settings)                 
            })
            .collect()
    }

    fn generate_clubs(
        country_id: u32,
        data: &DatabaseEntity,
        player_generator: &mut PlayerGenerator,
        staff_generator: &mut StaffGenerator,
    ) -> Vec<Club> {
        data
            .clubs
            .iter()
            .filter(|c| c.country_id == country_id)
            .map(|club| Club {
                id: club.id,
                name: club.name.clone(),
                location: Location {
                    city_id: club.location.city_id,
                },
                board: ClubBoard::new(),
                status: ClubStatus::Professional,
                finance: ClubFinances::new(club.finance.balance, Vec::new()),
                academy: ClubAcademy::new(100),
                teams: TeamCollection::new(
                    club.teams
                        .iter()
                        .map(|t| {
                            Team::new(
                                t.id,
                                t.league_id,
                                club.id,
                                t.name.clone(),
                                t.slug.clone(),
                                TeamType::from_str(&t.team_type).unwrap(),
                                TrainingSchedule::new(
                                    NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                                    NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                                ),
                                TeamReputation::new(
                                    t.reputation.home,
                                    t.reputation.national,
                                    t.reputation.world,
                                ),
                                PlayerCollection::new(Self::generate_players(
                                    player_generator,
                                    country_id,
                                )),
                                StaffCollection::new(
                                    Self::generate_staffs(staff_generator, country_id)),
                            )
                        })
                        .collect(),
                ),
            })
            .collect()
    }

    fn generate_players(player_generator: &mut PlayerGenerator, country_id: u32) -> Vec<Player> {
        let mut players = Vec::with_capacity(100);

        let mut goalkeepers: Vec<Player> = (0..IntegerUtils::random(3, 5))
            .map(|_| player_generator.generate(country_id, PositionType::Goalkeeper))
            .collect();

        let mut defenders: Vec<Player> = (0..IntegerUtils::random(20, 40))
            .map(|_| player_generator.generate(country_id, PositionType::Defender))
            .collect();

        let mut midfielders: Vec<Player> = (0..IntegerUtils::random(25, 35))
            .map(|_| player_generator.generate(country_id, PositionType::Midfielder))
            .collect();

        let mut strikers: Vec<Player> = (0..IntegerUtils::random(20, 24))
            .map(|_| player_generator.generate(country_id, PositionType::Striker))
            .collect();

        players.append(&mut goalkeepers);
        players.append(&mut defenders);
        players.append(&mut midfielders);
        players.append(&mut strikers);

        players
    }

    fn generate_staffs(staff_generator: &mut StaffGenerator, country_id: u32) -> Vec<Staff> {
        let mut staffs = Vec::with_capacity(30);

        staffs.push(staff_generator.generate(country_id, StaffPosition::DirectorOfFootball));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Director));

        staffs.push(staff_generator.generate(country_id, StaffPosition::AssistantManager));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach));

        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio));

        staffs
    }
}
