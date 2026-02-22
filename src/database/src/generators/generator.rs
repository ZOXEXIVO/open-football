use crate::generators::{PlayerGenerator, PositionType, StaffGenerator};
use crate::loaders::ContinentEntity;
use crate::{DatabaseEntity, ForeignPlayerEntry};
use core::PeopleNameGeneratorData;
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
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
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
                        names.nicknames.clone(),
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
                                    t.league_id,
                                    data,
                                )))
                                .staffs(StaffCollection::new(
                                    Self::generate_staffs(staff_generator, country_id, team_rep, &TeamType::from_str(&t.team_type).unwrap())
                                ))
                                .build()
                                .expect("Failed to build Team")
                        })
                        .collect(),
                ),
            })
            .collect()
    }

    fn generate_players(
        player_generator: &mut PlayerGenerator,
        country_id: u32,
        team_reputation: u16,
        team_type: &TeamType,
        league_id: Option<u32>,
        data: &DatabaseEntity,
    ) -> Vec<Player> {
        let mut players = Vec::with_capacity(100);

        // Age range based on team type
        let (min_age, max_age) = match team_type {
            TeamType::U18 => (15, 18),
            TeamType::U19 => (15, 19),
            TeamType::U20 => (16, 20),
            TeamType::U21 => (16, 21),
            TeamType::U23 => (17, 23),
            TeamType::B => (17, 28),
            TeamType::Main => (17, 35),
        };

        let is_youth = matches!(team_type, TeamType::U18 | TeamType::U19);

        let foreign_players: &[ForeignPlayerEntry] = league_id
            .and_then(|lid| data.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.foreign_players.as_slice())
            .unwrap_or(&[]);

        let total_foreign_weight: i32 = foreign_players.iter().map(|fp| fp.weight as i32).sum();

        let mut generate_one = |pos: PositionType| -> Player {
            if total_foreign_weight > 0 {
                let roll = IntegerUtils::random(0, 100);
                if roll < total_foreign_weight {
                    // Pick a foreign country via weighted random walk
                    let mut acc = 0i32;
                    for fp in foreign_players {
                        acc += fp.weight as i32;
                        if roll < acc {
                            let names = data.names_by_country.iter().find(|n| n.country_id == fp.country_id);
                            let people_names = match names {
                                Some(n) => PeopleNameGeneratorData {
                                    first_names: n.first_names.clone(),
                                    last_names: n.last_names.clone(),
                                    nicknames: n.nicknames.clone(),
                                },
                                None => PeopleNameGeneratorData {
                                    first_names: Vec::new(),
                                    last_names: Vec::new(),
                                    nicknames: Vec::new(),
                                },
                            };
                            let mut foreign_gen = PlayerGenerator::with_people_names(&people_names);
                            return foreign_gen.generate(fp.country_id, pos, team_reputation, min_age, max_age, is_youth);
                        }
                    }
                }
            }
            player_generator.generate(country_id, pos, team_reputation, min_age, max_age, is_youth)
        };

        for _ in 0..IntegerUtils::random(3, 5) {
            players.push(generate_one(PositionType::Goalkeeper));
        }
        for _ in 0..IntegerUtils::random(4, 8) {
            players.push(generate_one(PositionType::Defender));
        }
        for _ in 0..IntegerUtils::random(7, 11) {
            players.push(generate_one(PositionType::Midfielder));
        }
        for _ in 0..IntegerUtils::random(2, 5) {
            players.push(generate_one(PositionType::Striker));
        }

        players
    }

    fn generate_staffs(staff_generator: &mut StaffGenerator, country_id: u32, team_reputation: u16, team_type: &TeamType) -> Vec<Staff> {
        let mut staffs = Vec::with_capacity(30);

        if *team_type == TeamType::Main {
            // Only main team gets directors
            staffs.push(staff_generator.generate(country_id, StaffPosition::DirectorOfFootball, team_reputation));
            staffs.push(staff_generator.generate(country_id, StaffPosition::Director, team_reputation));
        }

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
