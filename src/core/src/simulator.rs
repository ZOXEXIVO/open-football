use crate::ai::{Ai, AiBatchProcessor};
use crate::club::ai::apply_ai_responses;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::{Continent, ContinentResult};
use crate::competitions::GlobalCompetitions;
use crate::league::LeagueTable;
use crate::r#match::MatchResult;
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::Player;
use chrono::{Duration, NaiveDateTime};
use std::time::Instant;

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
        crate::competitions::simulation::GlobalCompetitionSimulator::simulate(data);

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

    pub global_competitions: GlobalCompetitions
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
            global_competitions
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data.init_league_tables();

        data
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
