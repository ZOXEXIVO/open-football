use crate::ai::{Ai, AiBatchProcessor};
use crate::club::ai::apply_ai_responses;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::{Continent, ContinentResult};
use crate::competitions::GlobalCompetitions;
use crate::league::LeagueTable;
use crate::r#match::MatchResult;
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::{Player, TeamInfo};
use chrono::{Duration, NaiveDateTime};
use std::collections::HashMap;
use std::time::Instant;

/// Lightweight country info for nationality lookups.
/// Covers ALL countries (not just simulation participants).
#[derive(Clone, Debug)]
pub struct CountryInfo {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
}

pub struct FootballSimulator;

impl FootballSimulator {
    pub async fn simulate(data: &mut SimulatorData) -> SimulationResult {
        let mut result = SimulationResult::new();

        let current_data = data.date;
        let now = Instant::now();

        let ctx = GlobalContext::new(SimulationContext::new(data.date), Ai::new());

        // Phase A: Simulate (AI requests built, not executed)
        let results: Vec<ContinentResult> = data
            .continents
            .iter_mut()
            .map(|continent| continent.simulate(ctx.with_continent(continent.id)))
            .collect();

        // Phase B: Collect & batch-execute all AI requests
        let all_requests = ctx.ai.drain();

        if !all_requests.is_empty() {
            let completed = AiBatchProcessor::execute(all_requests).await;
            apply_ai_responses(completed, data);
        }

        // Phase C: Process results
        for continent_result in results {
            continent_result.process(data, &mut result);
        }

        // Global competitions
        //crate::competitions::simulation::GlobalCompetitionSimulator::simulate(data);

        // Refresh player indexes after transfers may have moved players between clubs
        if let Some(mut indexes) = data.indexes.take() {
            indexes.refresh_player_indexes(data);
            data.indexes = Some(indexes);
        }

        data.next_date();

        log::info!("simulate date {}, {}ms", current_data, now.elapsed().as_millis());

        result
    }
}

#[derive(Clone)]
pub struct SimulatorData {
    pub continents: Vec<Continent>,

    pub date: NaiveDateTime,

    pub transfer_pool: TransferPool<Player>,

    pub indexes: Option<SimulatorDataIndexes>,

    pub match_played: bool,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,

    /// All countries by id (for nationality lookups — includes countries without active leagues)
    pub country_info: HashMap<u32, CountryInfo>,

    /// Global match result storage — all match types (league, cup, national team) write here
    pub match_store: crate::league::MatchStorage,
}

impl SimulatorData {
    pub fn new(date: NaiveDateTime, continents: Vec<Continent>, global_competitions: GlobalCompetitions) -> Self {
        // Build country_info from simulation participants
        let country_info: HashMap<u32, CountryInfo> = continents.iter()
            .flat_map(|cont| &cont.countries)
            .map(|c| (c.id, CountryInfo {
                id: c.id,
                code: c.code.clone(),
                slug: c.slug.clone(),
                name: c.name.clone(),
            }))
            .collect();

        let mut data = SimulatorData {
            continents,
            date,
            transfer_pool: TransferPool::new(),
            indexes: None,
            match_played: false,
            watchlist: Vec::new(),
            global_competitions,
            country_info,
            match_store: crate::league::MatchStorage::new(),
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data.init_league_tables();
        data.seed_player_histories();

        data
    }

    /// Register country info for countries that may not have active leagues in the simulation.
    /// Called by the database generator to ensure nationality lookups always succeed.
    pub fn add_country_info(&mut self, id: u32, code: String, slug: String, name: String) {
        self.country_info.entry(id).or_insert(CountryInfo { id, code, slug, name });
    }

    /// Initialize all league tables with their teams so tables are populated from the start.
    fn init_league_tables(&mut self) {
        for continent in &mut self.continents {
            for country in &mut continent.countries {
                let clubs = &country.clubs;

                for league in &mut country.leagues.leagues {
                    if !league.table.rows.is_empty() {
                        continue;
                    }

                    let team_ids: Vec<u32> = clubs
                        .iter()
                        .flat_map(|c| c.teams.with_league(league.id))
                        .collect();

                    if !team_ids.is_empty() {
                        league.table = LeagueTable::new(&team_ids);
                    }
                }
            }
        }
    }

    /// Seed statistics history for all players that have no history yet.
    fn seed_player_histories(&mut self) {
        let date = self.date.date();

        for continent in &mut self.continents {
            for country in &mut continent.countries {
                // Build league lookup: league_id -> (name, slug)
                let league_lookup: HashMap<u32, (String, String)> = country.leagues.leagues.iter()
                    .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
                    .collect();

                for club in &mut country.clubs {
                    // Get main team info — used for all teams in player history
                    let main_team_info: Option<(String, String, u16)> = club.teams.teams.iter()
                        .find(|t| t.team_type == crate::TeamType::Main)
                        .map(|t| (t.name.clone(), t.slug.clone(), t.reputation.world));

                    let main_team_league = club.teams.teams.iter()
                        .find(|t| t.team_type == crate::TeamType::Main)
                        .and_then(|t| t.league_id)
                        .and_then(|lid| league_lookup.get(&lid))
                        .cloned()
                        .unwrap_or_default();

                    for team in &mut club.teams.teams {
                        let (team_name, team_slug, team_reputation) = match (&team.team_type, &main_team_info) {
                            (crate::TeamType::Main, _) | (_, None) => {
                                (team.name.clone(), team.slug.clone(), team.reputation.world)
                            }
                            (_, Some((name, slug, rep))) => {
                                (name.clone(), slug.clone(), *rep)
                            }
                        };

                        let team_info = TeamInfo {
                            name: team_name,
                            slug: team_slug,
                            reputation: team_reputation,
                            league_name: main_team_league.0.clone(),
                            league_slug: main_team_league.1.clone(),
                        };

                        for player in &mut team.players.players {
                            player.statistics_history.seed_initial_team(&team_info, date);
                        }
                    }
                }
            }
        }
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }
}

pub struct SimulationResult {
    pub match_results: Vec<MatchResult>,
}

impl Default for SimulationResult {
    fn default() -> Self {
        Self::new()
    }
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
