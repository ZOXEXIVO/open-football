use crate::generators::{PlayerGenerator, PositionType, StaffGenerator};
use crate::loaders::ContinentEntity;
use crate::{DatabaseEntity, ForeignPlayerEntry};
use core::PeopleNameGeneratorData;
use chrono::{NaiveDate, NaiveDateTime};
use core::club::academy::ClubAcademy;
use core::context::NaiveTime;
use core::continent::Continent;
use core::competitions::GlobalCompetitions;
use core::league::LeagueCollection;
use core::league::{DayMonthPeriod, League, LeagueSettings};
use core::shared::Location;
use core::utils::IntegerUtils;
use core::ClubStatus;
use core::TeamCollection;
use crate::generators::convert::convert_national_competition;
use core::{
    Club, ClubBoard, ClubColors, ClubFinances, Country, CountryGeneratorData, CountryPricing, CountrySettings, Player,
    PlayerCollection, SimulatorData, Staff, StaffCollection, StaffPosition, Team,
    TeamReputation, TeamType, TrainingSchedule,
    CompetitionScope, NationalCompetitionConfig,
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

        // Convert all national competition entities to runtime configs
        let all_configs: Vec<NationalCompetitionConfig> = data
            .national_competitions
            .iter()
            .map(|e| convert_national_competition(e))
            .collect();

        // Separate global configs for GlobalCompetitions
        let global_configs: Vec<NationalCompetitionConfig> = all_configs
            .iter()
            .filter(|c| c.scope == CompetitionScope::Global)
            .cloned()
            .collect();

        let global_competitions = GlobalCompetitions::new(global_configs);

        let continents = data
            .continents
            .iter()
            .map(|continent| {
                // Filter configs relevant to this continent:
                // - continental configs where continent_id matches
                // - global configs that have a qualifying zone for this continent
                let continent_configs: Vec<NationalCompetitionConfig> = all_configs
                    .iter()
                    .filter(|config| {
                        match config.scope {
                            CompetitionScope::Continental => {
                                config.continent_id == Some(continent.id)
                            }
                            CompetitionScope::Global => {
                                config.qualifying.zones.iter().any(|z| z.continent_id == continent.id)
                            }
                        }
                    })
                    .cloned()
                    .collect();

                Continent::new(
                    continent.id,
                    continent.name.clone(),
                    DatabaseGenerator::generate_countries(continent, data),
                    continent_configs,
                )
            }).collect();

        SimulatorData::new(current_date, continents, global_competitions)
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

                let mut clubs = DatabaseGenerator::generate_clubs(
                    country.id,
                    data,
                    &mut player_generator,
                    &mut staff_generator,
                );

                let mut leagues_vec = DatabaseGenerator::generate_leagues(country.id, data);
                DatabaseGenerator::create_youth_leagues(country.id, &mut clubs, &mut leagues_vec);
                DatabaseGenerator::create_friendly_leagues(country.id, &mut clubs, &mut leagues_vec);
                let leagues = LeagueCollection::new(leagues_vec);

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
                    tier: league.tier,
                    promotion_spots: league.promotion_spots,
                    relegation_spots: league.relegation_spots,
                };

                League::new(league.id, league.name.clone(), league.slug.clone(), league.country_id, league.reputation, settings, false)
            })
            .collect()
    }

    fn create_youth_leagues(country_id: u32, clubs: &mut [Club], leagues: &mut Vec<League>) {
        // Build a map: club_id → parent league_id (from the club's Main team)
        let club_league_map: Vec<(u32, u32)> = clubs
            .iter()
            .filter_map(|club| {
                let main_league_id = club.teams.teams
                    .iter()
                    .find(|t| t.team_type == TeamType::Main)
                    .and_then(|t| t.league_id)?;
                Some((club.id, main_league_id))
            })
            .collect();

        // Snapshot parent leagues to create one U18 league per parent
        let parent_leagues: Vec<(u32, String, String, u16, LeagueSettings)> = leagues
            .iter()
            .map(|l| (l.id, l.name.clone(), l.slug.clone(), l.reputation, l.settings.clone()))
            .collect();

        for (parent_id, parent_name, parent_slug, parent_rep, parent_settings) in &parent_leagues {
            // Check if any club in this parent league has a U18 team
            let has_u18 = clubs.iter().any(|club| {
                club_league_map.iter().any(|(cid, lid)| *cid == club.id && lid == parent_id)
                    && club.teams.teams.iter().any(|t| t.team_type == TeamType::U18)
            });

            if !has_u18 {
                continue;
            }

            let youth_league_id = parent_id + 100000;
            let youth_reputation = (parent_rep / 10).max(100);

            let youth_settings = LeagueSettings {
                season_starting_half: parent_settings.season_starting_half,
                season_ending_half: parent_settings.season_ending_half,
                tier: 99,
                promotion_spots: 0,
                relegation_spots: 0,
            };

            let youth_league = League::new(
                youth_league_id,
                format!("{} U18", parent_name),
                format!("{}-u18", parent_slug),
                country_id,
                youth_reputation,
                youth_settings,
                true,
            );

            leagues.push(youth_league);

            // Assign U18 teams to this youth league based on their club's parent league
            for club in clubs.iter_mut() {
                let is_in_parent = club_league_map.iter().any(|(cid, lid)| *cid == club.id && lid == parent_id);
                if !is_in_parent {
                    continue;
                }
                for team in &mut club.teams.teams {
                    if team.team_type == TeamType::U18 {
                        team.league_id = Some(youth_league_id);
                    }
                }
            }
        }
    }

    fn create_friendly_leagues(country_id: u32, clubs: &mut [Club], leagues: &mut Vec<League>) {
        // Build a map: club_id → parent league_id (from the club's Main team)
        let club_league_map: Vec<(u32, u32)> = clubs
            .iter()
            .filter_map(|club| {
                let main_league_id = club.teams.teams
                    .iter()
                    .find(|t| t.team_type == TeamType::Main)
                    .and_then(|t| t.league_id)?;
                Some((club.id, main_league_id))
            })
            .collect();

        let parent_leagues: Vec<(u32, String, String, u16, LeagueSettings)> = leagues
            .iter()
            .filter(|l| !l.friendly)
            .map(|l| (l.id, l.name.clone(), l.slug.clone(), l.reputation, l.settings.clone()))
            .collect();

        for (parent_id, parent_name, parent_slug, parent_rep, parent_settings) in &parent_leagues {
            // Find clubs in this parent league that have teams without a league assignment
            let has_unassigned = clubs.iter().any(|club| {
                club_league_map.iter().any(|(cid, lid)| *cid == club.id && lid == parent_id)
                    && club.teams.teams.iter().any(|t| t.league_id.is_none())
            });

            if !has_unassigned {
                continue;
            }

            let friendly_league_id = parent_id + 200000;
            let friendly_reputation = (parent_rep / 10).max(100);

            let friendly_settings = LeagueSettings {
                season_starting_half: parent_settings.season_starting_half,
                season_ending_half: parent_settings.season_ending_half,
                tier: 99,
                promotion_spots: 0,
                relegation_spots: 0,
            };

            let friendly_league = League::new(
                friendly_league_id,
                format!("{} Reserves", parent_name),
                format!("{}-reserves", parent_slug),
                country_id,
                friendly_reputation,
                friendly_settings,
                true,
            );

            leagues.push(friendly_league);

            // Assign teams without league_id to this friendly league
            for club in clubs.iter_mut() {
                let is_in_parent = club_league_map.iter().any(|(cid, lid)| *cid == club.id && lid == parent_id);
                if !is_in_parent {
                    continue;
                }
                for team in &mut club.teams.teams {
                    if team.league_id.is_none() {
                        team.league_id = Some(friendly_league_id);
                    }
                }
            }
        }
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
                            let team_type = TeamType::from_str(&t.team_type).unwrap();

                            let team_name = match &team_type {
                                TeamType::Main => t.name.clone(),
                                _ => format!("{} {}", t.name, team_type),
                            };

                            let players = PlayerCollection::new(Self::generate_players(
                                player_generator,
                                country_id,
                                team_rep,
                                &team_type,
                                t.league_id,
                                data,
                            ));

                            let staffs = StaffCollection::new(
                                Self::generate_staffs(staff_generator, country_id, team_rep, &team_type)
                            );

                            Team::builder()
                                .id(t.id)
                                .league_id(t.league_id)
                                .club_id(club.id)
                                .name(team_name)
                                .slug(t.slug.clone())
                                .team_type(team_type)
                                .training_schedule(TrainingSchedule::new(
                                    NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
                                    NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                                ))
                                .reputation(TeamReputation::new(
                                    t.reputation.home,
                                    t.reputation.national,
                                    t.reputation.world,
                                ))
                                .players(players)
                                .staffs(staffs)
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
            TeamType::B | TeamType::Reserve => (17, 28),
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
