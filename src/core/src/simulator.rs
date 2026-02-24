use crate::context::{GlobalContext, SimulationContext};
use crate::continent::{Continent, ContinentResult};
use crate::global_competitions::GlobalCompetitions;
use crate::league::League;
use crate::r#match::MatchResult;
use crate::shared::{SimulatorDataIndexes, TeamData};
use crate::transfers::TransferPool;
use crate::utils::Logging;
use crate::{Club, Country, Player, Staff, Team};
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

                // Global competitions: assembly + simulation + phase transitions
                let date = data.date.date();
                data.global_competitions.check_tournament_assembly(date, &data.continents);
                Self::simulate_global_competitions(data, date);
                data.global_competitions.check_phase_transitions();

                data.next_date();
            },
            &format!("simulate date {}", current_data),
        );

        result
    }

    fn simulate_global_competitions(data: &mut SimulatorData, date: chrono::NaiveDate) {
        use crate::NationalTeam;
        use log::info;
        use rayon::iter::{IntoParallelIterator, ParallelIterator};
        use std::collections::HashMap;

        let todays_matches = data.global_competitions.get_todays_matches(date);
        if todays_matches.is_empty() {
            return;
        }

        // Build squads - need to search across all continents
        let prepared: Vec<(usize, crate::r#match::MatchSquad, crate::r#match::MatchSquad)> = todays_matches
            .iter()
            .enumerate()
            .filter_map(|(idx, fixture)| {
                let home = Self::build_global_match_squad(&mut data.continents, fixture.home_country_id, date)?;
                let away = Self::build_global_match_squad(&mut data.continents, fixture.away_country_id, date)?;
                Some((idx, home, away))
            })
            .collect();

        // Run match engines in parallel
        let engine_results: Vec<(usize, u8, u8, HashMap<u32, u16>)> = prepared
            .into_par_iter()
            .map(|(idx, home_squad, away_squad)| {
                let (home_score, away_score, player_goals) =
                    NationalTeam::play_competition_match(home_squad, away_squad);
                (idx, home_score, away_score, player_goals)
            })
            .collect();

        // Apply results
        for (fixture_idx, home_score, away_score, _player_goals) in engine_results {
            let fixture = &todays_matches[fixture_idx];
            let home_country_id = fixture.home_country_id;
            let away_country_id = fixture.away_country_id;

            let penalty_winner = if fixture.phase.is_knockout() && home_score == away_score {
                let home_rep = Self::get_global_country_reputation(&data.continents, home_country_id);
                let away_rep = Self::get_global_country_reputation(&data.continents, away_country_id);
                if home_rep >= away_rep {
                    Some(home_country_id)
                } else {
                    Some(away_country_id)
                }
            } else {
                None
            };

            data.global_competitions.record_result(
                fixture,
                home_score,
                away_score,
                penalty_winner,
            );

            // Update Elo ratings across continents
            let away_elo = Self::get_global_country_elo(&data.continents, away_country_id);
            let home_elo = Self::get_global_country_elo(&data.continents, home_country_id);

            for continent in &mut data.continents {
                if let Some(country) = continent.countries.iter_mut().find(|c| c.id == home_country_id) {
                    country.national_team.update_elo(home_score, away_score, away_elo);
                }
                if let Some(country) = continent.countries.iter_mut().find(|c| c.id == away_country_id) {
                    country.national_team.update_elo(away_score, home_score, home_elo);
                }
            }

            let label = data.global_competitions
                .tournaments
                .get(fixture.tournament_idx)
                .map(|t| t.short_name())
                .unwrap_or("INT");

            let home_name = Self::get_global_country_name(&data.continents, home_country_id);
            let away_name = Self::get_global_country_name(&data.continents, away_country_id);

            info!(
                "Global competition ({}): {} vs {} - {}:{}",
                label, home_name, away_name, home_score, away_score
            );
        }
    }

    fn build_global_match_squad(
        continents: &mut [Continent],
        country_id: u32,
        date: chrono::NaiveDate,
    ) -> Option<crate::r#match::MatchSquad> {
        for continent in continents.iter_mut() {
            if continent.countries.iter().any(|c| c.id == country_id) {
                return continent.build_country_match_squad(country_id, date);
            }
        }
        None
    }

    fn get_global_country_reputation(continents: &[Continent], country_id: u32) -> u16 {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.reputation)
            .unwrap_or(0)
    }

    fn get_global_country_elo(continents: &[Continent], country_id: u32) -> u16 {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.national_team.elo_rating)
            .unwrap_or(1500)
    }

    fn get_global_country_name(continents: &[Continent], country_id: u32) -> String {
        continents
            .iter()
            .flat_map(|c| &c.countries)
            .find(|c| c.id == country_id)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("Country {}", country_id))
    }
}

pub struct SimulatorData {
    pub continents: Vec<Continent>,

    pub date: NaiveDateTime,

    pub transfer_pool: TransferPool<Player>,

    pub indexes: Option<SimulatorDataIndexes>,

    pub match_played: bool,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,
}

impl SimulatorData {
    pub fn new(date: NaiveDateTime, continents: Vec<Continent>, global_competitions: GlobalCompetitions) -> Self {
        let mut data = SimulatorData {
            continents,
            date,
            transfer_pool: TransferPool::new(),
            indexes: None,
            match_played: false,
            watchlist: Vec::new(),
            global_competitions,
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
        // Fast path: indexed lookup
        if let Some((player_continent_id, player_country_id, player_club_id, player_team_id)) =
            self.indexes.as_ref().and_then(|indexes| indexes.get_player_location(id))
        {
            let found = self
                .continent(player_continent_id)
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
                .and_then(|team| team.players.players.iter().find(|c| c.id == id));

            if found.is_some() {
                return found;
            }
        }

        // Fallback: brute-force scan (index may be stale after transfers)
        self.player_brute_force(id)
    }

    pub fn player_with_team(&self, player_id: u32) -> Option<(&Player, &Team)> {
        // Fast path: indexed lookup
        if let Some((continent_id, country_id, club_id, team_id)) =
            self.indexes.as_ref().and_then(|idx| idx.get_player_location(player_id))
        {
            let result = self
                .continent(continent_id)
                .and_then(|c| c.countries.iter().find(|co| co.id == country_id))
                .and_then(|co| co.clubs.iter().find(|cl| cl.id == club_id))
                .and_then(|cl| cl.teams.teams.iter().find(|t| t.id == team_id))
                .and_then(|team| {
                    team.players.players.iter().find(|p| p.id == player_id).map(|p| (p, team))
                });

            if result.is_some() {
                return result;
            }
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(player) = team.players.players.iter().find(|p| p.id == player_id) {
                            return Some((player, team));
                        }
                    }
                }
            }
        }
        None
    }

    fn player_brute_force(&self, id: u32) -> Option<&Player> {
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(player) = team.players.players.iter().find(|p| p.id == id) {
                            return Some(player);
                        }
                    }
                }
            }
        }
        None
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

    pub fn staff_with_team(&self, staff_id: u32) -> Option<(&Staff, &Team)> {
        // Fast path: indexed lookup
        if let Some((continent_id, country_id, club_id, team_id)) =
            self.indexes.as_ref().and_then(|idx| idx.get_staff_location(staff_id))
        {
            let result = self
                .continent(continent_id)
                .and_then(|c| c.countries.iter().find(|co| co.id == country_id))
                .and_then(|co| co.clubs.iter().find(|cl| cl.id == club_id))
                .and_then(|cl| cl.teams.teams.iter().find(|t| t.id == team_id))
                .and_then(|team| {
                    team.staffs.staffs.iter().find(|s| s.id == staff_id).map(|s| (s, team))
                });

            if result.is_some() {
                return result;
            }
        }

        // Fallback: brute-force
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if let Some(staff) = team.staffs.staffs.iter().find(|s| s.id == staff_id) {
                            return Some((staff, team));
                        }
                    }
                }
            }
        }
        None
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
