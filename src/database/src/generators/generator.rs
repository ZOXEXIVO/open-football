use crate::generators::{PlayerGenerator, PositionType, StaffGenerator};
use crate::loaders::ContinentEntity;
use crate::DatabaseEntity;
use chrono::{NaiveDate, NaiveDateTime};
use core::club::academy::ClubAcademy;
use core::context::NaiveTime;
use core::continent::Continent;
use core::league::LeagueCollection;
use core::league::{DayMonthPeriod, League, LeagueSettings};
use core::shared::Location;
use core::utils::IntegerUtils;
use core::ClubStatus;
use core::TeamCollection;
use core::{
    Club, ClubBoard, ClubColors, ClubFinances, Country, CountryGeneratorData, CountryPricing, CountrySettings, Player,
    PlayerCollection, SimulatorData, Staff, StaffCollection, StaffPosition, Team,
    TeamReputation, TeamType, TrainingSchedule,
};
use core::transfers::pipeline::ClubTransferPlan;
use std::str::FromStr;

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

                let leagues = LeagueCollection::new(
                    DatabaseGenerator::generate_leagues(country.id, data)
                );

                let settings = CountrySettings {
                    pricing: CountryPricing {
                        price_level: country.settings.pricing.price_level,
                    },
                };

                Country::builder()
                    .id(country.id)
                    .code(country.code.clone())
                    .slug(country.slug.clone())
                    .name(country.name.clone())
                    .background_color(country.background_color.clone())
                    .foreground_color(country.foreground_color.clone())
                    .continent_id(continent.id)
                    .leagues(leagues)
                    .clubs(clubs)
                    .reputation(country.reputation)
                    .settings(settings)
                    .generator_data(generator_data)
                    .build()
                    .expect("Failed to build Country")
            }).collect()
    }

    fn generate_leagues(country_id: u32, data: &DatabaseEntity) -> Vec<League> {
        data
            .leagues
            .iter()
            .filter(|l| l.country_id == country_id)
            .map(|league| {
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
                colors: ClubColors {
                    background: club.colors.background.clone(),
                    foreground: club.colors.foreground.clone(),
                },
                transfer_plan: ClubTransferPlan::new(),
                teams: TeamCollection::new(
                    club.teams
                        .iter()
                        .map(|t| {
                            let team_rep = t.reputation.world;

                            Team::builder()
                                .id(t.id)
                                .league_id(t.league_id)
                                .club_id(club.id)
                                .name(t.name.clone())
                                .slug(t.slug.clone())
                                .team_type(TeamType::from_str(&t.team_type).unwrap())
                                .training_schedule(TrainingSchedule::new(
                                    NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                                    NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                                ))
                                .reputation(TeamReputation::new(
                                    t.reputation.home,
                                    t.reputation.national,
                                    t.reputation.world,
                                ))
                                .players(PlayerCollection::new(Self::generate_players(
                                    player_generator,
                                    country_id,
                                    team_rep,
                                    &TeamType::from_str(&t.team_type).unwrap(),
                                )))
                                .staffs(StaffCollection::new(
                                    Self::generate_staffs(staff_generator, country_id, team_rep)
                                ))
                                .build()
                                .expect("Failed to build Team")
                        })
                        .collect(),
                ),
            })
            .collect()
    }

    fn generate_players(player_generator: &mut PlayerGenerator, country_id: u32, team_reputation: u16, team_type: &TeamType) -> Vec<Player> {
        let mut players = Vec::with_capacity(100);

        // Age range based on team type
        let (min_age, max_age) = match team_type {
            TeamType::U18 => (15, 18),
            TeamType::U19 => (15, 19),
            TeamType::U21 => (16, 21),
            TeamType::U23 => (17, 23),
            TeamType::B => (17, 28),
            TeamType::Main => (17, 35),
        };

        let is_youth = matches!(team_type, TeamType::U18 | TeamType::U19);

        let mut goalkeepers: Vec<Player> = (0..IntegerUtils::random(3, 5))
            .map(|_| player_generator.generate(country_id, PositionType::Goalkeeper, team_reputation, min_age, max_age, is_youth))
            .collect();

        let mut defenders: Vec<Player> = (0..IntegerUtils::random(4, 8))
            .map(|_| player_generator.generate(country_id, PositionType::Defender, team_reputation, min_age, max_age, is_youth))
            .collect();

        let mut midfielders: Vec<Player> = (0..IntegerUtils::random(7, 11))
            .map(|_| player_generator.generate(country_id, PositionType::Midfielder, team_reputation, min_age, max_age, is_youth))
            .collect();

        let mut strikers: Vec<Player> = (0..IntegerUtils::random(2, 5))
            .map(|_| player_generator.generate(country_id, PositionType::Striker, team_reputation, min_age, max_age, is_youth))
            .collect();

        players.append(&mut goalkeepers);
        players.append(&mut defenders);
        players.append(&mut midfielders);
        players.append(&mut strikers);

        players
    }

    fn generate_staffs(staff_generator: &mut StaffGenerator, country_id: u32, team_reputation: u16) -> Vec<Staff> {
        let mut staffs = Vec::with_capacity(30);

        staffs.push(staff_generator.generate(country_id, StaffPosition::DirectorOfFootball, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Director, team_reputation));

        staffs.push(staff_generator.generate(country_id, StaffPosition::AssistantManager, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Coach, team_reputation));

        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio, team_reputation));
        staffs.push(staff_generator.generate(country_id, StaffPosition::Physio, team_reputation));

        staffs
    }
}
