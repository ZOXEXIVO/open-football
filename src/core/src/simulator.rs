use crate::context::{GlobalContext, SimulationContext};
use crate::continent::{Continent, ContinentResult};
use crate::league::League;
use crate::r#match::MatchResult;
use crate::shared::{SimulatorDataIndexes, TeamData};
use crate::transfers::TransferPool;
use crate::utils::Logging;
use crate::{Club, Country, Player, Team};
use chrono::{Duration, NaiveDateTime};

pub struct FootballSimulator;

impl FootballSimulator {
    pub fn simulate(data: &mut SimulatorData) -> SimulationResult {
        let mut result = SimulationResult::new();

        let current_data = data.date;

        Logging::estimate(
            || {
                let ctx = GlobalContext::new(SimulationContext::new(data.date));

                let results: Vec<ContinentResult> = data
                    .continents
                    .iter_mut()
                    .map(|continent| continent.simulate(ctx.with_continent(continent.id)))
                    .collect();

                for continent_result in results {
                    continent_result.process(data, &mut result);
                }

                data.next_date();
            },
            &format!("simulate date {}", current_data),
        );

        result
    }
}

pub struct SimulatorData {
    pub continents: Vec<Continent>,

    pub date: NaiveDateTime,

    pub transfer_pool: TransferPool<Player>,

    pub indexes: Option<SimulatorDataIndexes>,

    pub match_played: bool
}

impl SimulatorData {
    pub fn new(date: NaiveDateTime, continents: Vec<Continent>) -> Self {
        let mut data = SimulatorData {
            continents,
            date,
            transfer_pool: TransferPool::new(),
            indexes: None,
            match_played: false
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }

    pub fn continent(&self, id: u32) -> Option<&Continent> {
        self.continents.iter().find(|c| c.id == id)
    }

    pub fn continent_mut(&mut self, id: u32) -> Option<&mut Continent> {
        self.continents.iter_mut().find(|c| c.id == id)
    }

    pub fn country(&self, id: u32) -> Option<&Country> {
        self.continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == id)
    }

    pub fn country_mut(&mut self, id: u32) -> Option<&mut Country> {
        self.continents
            .iter_mut()
            .flat_map(|c| &mut c.countries)
            .find(|c| c.id == id)
    }

    pub fn league(&self, id: u32) -> Option<&League> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_league_location(id))
            .and_then(|(league_continent_id, league_country_id)| {
                self.continent(league_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter()
                            .find(|country| country.id == league_country_id)
                    })
                    .and_then(|country| {
                        country
                            .leagues
                            .leagues
                            .iter()
                            .find(|league| league.id == id)
                    })
            })
    }

    pub fn league_mut(&mut self, id: u32) -> Option<&mut League> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_league_location(id))
            .and_then(|(league_continent_id, league_country_id)| {
                self.continent_mut(league_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter_mut()
                            .find(|country| country.id == league_country_id)
                    })
                    .and_then(|country| {
                        country
                            .leagues
                            .leagues
                            .iter_mut()
                            .find(|league| league.id == id)
                    })
            })
    }

    pub fn team_data(&self, id: u32) -> Option<&TeamData> {
        self.indexes.as_ref().unwrap().get_team_data(id)
    }

    pub fn country_by_club(&self, club_id: u32) -> Option<&Country> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(club_id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == club_country_id)
                })
            })
    }

    pub fn club(&self, id: u32) -> Option<&Club> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter()
                        .find(|country| country.id == club_country_id)
                })
            })
            .and_then(|country| country.clubs.iter().find(|club| club.id == id))
    }

    pub fn club_mut(&mut self, id: u32) -> Option<&mut Club> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_club_location(id))
            .and_then(|(club_continent_id, club_country_id)| {
                self.continent_mut(club_continent_id).and_then(|continent| {
                    continent
                        .countries
                        .iter_mut()
                        .find(|country| country.id == club_country_id)
                })
            })
            .and_then(|country| country.clubs.iter_mut().find(|club| club.id == id))
    }

    pub fn team(&self, id: u32) -> Option<&Team> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_team_location(id))
            .and_then(|(team_continent_id, team_country_id, team_club_id)| {
                self.continent(team_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter()
                            .find(|country| country.id == team_country_id)
                    })
                    .and_then(|country| country.clubs.iter().find(|club| club.id == team_club_id))
                    .and_then(|club| club.teams.teams.iter().find(|team| team.id == id))
            })
    }

    pub fn team_mut(&mut self, id: u32) -> Option<&mut Team> {
        self.indexes
            .as_ref()
            .and_then(|indexes| indexes.get_team_location(id))
            .and_then(|(team_continent_id, team_country_id, team_club_id)| {
                self.continent_mut(team_continent_id)
                    .and_then(|continent| {
                        continent
                            .countries
                            .iter_mut()
                            .find(|country| country.id == team_country_id)
                    })
                    .and_then(|country| {
                        country
                            .clubs
                            .iter_mut()
                            .find(|club| club.id == team_club_id)
                    })
                    .and_then(|club| club.teams.teams.iter_mut().find(|team| team.id == id))
            })
    }

    pub fn player(&self, id: u32) -> Option<&Player> {
        let (player_continent_id, player_country_id, player_club_id, player_team_id) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.get_player_location(id))?;

        self.continent(player_continent_id)
            .and_then(|continent| {
                continent
                    .countries
                    .iter()
                    .find(|country| country.id == player_country_id)
            })
            .and_then(|country| country.clubs.iter().find(|club| club.id == player_club_id))
            .and_then(|club| {
                club.teams
                    .teams
                    .iter()
                    .find(|team| team.id == player_team_id)
            })
            .and_then(|team| team.players.players.iter().find(|c| c.id == id))
    }

    pub fn player_mut(&mut self, id: u32) -> Option<&mut Player> {
        let (player_continent_id, player_country_id, player_club_id, player_team_id) = self
            .indexes
            .as_ref()
            .and_then(|indexes| indexes.get_player_location(id))?;

        self.continent_mut(player_continent_id)
            .and_then(|continent| {
                continent
                    .countries
                    .iter_mut()
                    .find(|country| country.id == player_country_id)
            })
            .and_then(|country| {
                country
                    .clubs
                    .iter_mut()
                    .find(|club| club.id == player_club_id)
            })
            .and_then(|club| {
                club.teams
                    .teams
                    .iter_mut()
                    .find(|team| team.id == player_team_id)
            })
            .and_then(|team| team.players.players.iter_mut().find(|c| c.id == id))
    }
}

pub struct SimulationResult {
    pub match_results: Vec<MatchResult>,
}

impl SimulationResult {
    pub fn new() -> Self {
        SimulationResult {
            match_results: Vec::new(),
        }
    }

    pub fn has_match_results(&self) -> bool {
        !self.match_results.is_empty()
    }
}
