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
use rayon::prelude::*;
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

        let current_date = data.date;
        let tick_start = Instant::now();

        let ctx = GlobalContext::new(SimulationContext::new(data.date), Ai::new());

        // Phase ordering note:
        // A simulates continents and emits AI requests as FnOnce closures that
        // capture stable IDs (club_id, player_id, …). B executes those requests
        // against the freshly-mutated data. C then drains the ContinentResults
        // collected in A. The results reference the same stable IDs, so Phase B
        // mutations (contracts, morale, etc.) are safely visible to Phase C.

        // Phase A: simulate all continents in parallel. Each call mutates its
        // own continent and pushes AI requests into the shared (Arc<Mutex>) Ai
        // collector. Continents are independent during this phase.
        let phase_a = Instant::now();
        let continent_ids: Vec<u32> = data.continents.iter().map(|c| c.id).collect();
        let results: Vec<ContinentResult> = data
            .continents
            .par_iter_mut()
            .zip(continent_ids.par_iter())
            .map(|(continent, &cid)| continent.simulate(ctx.with_continent(cid)))
            .collect();
        let phase_a_ms = phase_a.elapsed().as_millis();

        // Phase B: collect and batch-execute all AI requests
        let phase_b = Instant::now();
        let all_requests = ctx.ai.drain();
        let ai_count = all_requests.len();
        if !all_requests.is_empty() {
            let completed = AiBatchProcessor::execute(all_requests).await;
            apply_ai_responses(completed, data);
        }
        let phase_b_ms = phase_b.elapsed().as_millis();

        // Phase C: process the collected results against post-AI data
        let phase_c = Instant::now();
        for continent_result in results {
            continent_result.process(data, &mut result);
        }
        let phase_c_ms = phase_c.elapsed().as_millis();

        // Global competitions (Champions League, World Cup, etc.)
        let phase_g = Instant::now();
        crate::competitions::simulation::GlobalCompetitionSimulator::simulate(data);
        let phase_g_ms = phase_g.elapsed().as_millis();

        // Refresh player indexes only if a transfer actually moved a player
        // between clubs today. Walking the world every day is wasteful.
        let refresh = Instant::now();
        let mut refresh_ms = 0u128;
        if data.dirty_player_index {
            if let Some(mut indexes) = data.indexes.take() {
                indexes.refresh_player_indexes(data);
                data.indexes = Some(indexes);
            }
            data.dirty_player_index = false;
            refresh_ms = refresh.elapsed().as_millis();
        }

        // Seed history for any players created today that haven't been seeded
        // (youth intake, regens, new clubs) — catches them within one tick.
        data.seed_missing_player_histories();

        data.next_date();

        log::info!(
            "simulate {} total={}ms (A={}ms, B={}ms [{} reqs], C={}ms, global={}ms, idx={}ms)",
            current_date,
            tick_start.elapsed().as_millis(),
            phase_a_ms,
            phase_b_ms,
            ai_count,
            phase_c_ms,
            phase_g_ms,
            refresh_ms,
        );

        result
    }
}

#[derive(Clone)]
pub struct SimulatorData {
    pub continents: Vec<Continent>,

    pub date: NaiveDateTime,

    pub transfer_pool: TransferPool<Player>,

    pub indexes: Option<SimulatorDataIndexes>,

    /// Set to true whenever a transfer moves a player between clubs. Checked
    /// by the simulator to decide whether to rebuild player location indexes.
    pub dirty_player_index: bool,

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
            dirty_player_index: false,
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

    /// Remove a country from the nationality lookup map.
    pub fn remove_country_info(&mut self, id: u32) {
        self.country_info.remove(&id);
    }

    /// Initial population of league tables at construction time.
    /// Per-season rebuilds happen inside `League::simulate` when a new
    /// schedule is generated — see `league/league.rs:119`. The skip-if-
    /// non-empty guard below is therefore intentional: it only prevents
    /// the initial seed from clobbering an already-populated table.
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

    /// Seed statistics history for every player (called on construction).
    fn seed_player_histories(&mut self) {
        self.seed_player_histories_inner(false);
    }

    /// Seed any players whose history is still empty — catches youth intake,
    /// regens, and newly-generated clubs within one simulated tick.
    pub fn seed_missing_player_histories(&mut self) {
        self.seed_player_histories_inner(true);
    }

    fn seed_player_histories_inner(&mut self, only_empty: bool) {
        let date = self.date.date();

        for continent in &mut self.continents {
            for country in &mut continent.countries {
                let league_lookup: HashMap<u32, (String, String)> = country.leagues.leagues.iter()
                    .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
                    .collect();

                for club in &mut country.clubs {
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
                            if only_empty && !player.statistics_history.is_empty() {
                                continue;
                            }
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
