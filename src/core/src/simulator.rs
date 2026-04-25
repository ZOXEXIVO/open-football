use crate::ai::{Ai, AiBatchProcessor};
use crate::club::ai::apply_ai_responses;
use crate::club::board::manager_market;
use crate::club::staff::free_pool;
use crate::competitions::simulation::GlobalCompetitionSimulator;
use crate::competitions::GlobalCompetitions;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::{Continent, ContinentResult};
use crate::league::{LeagueTable, MatchStorage};
use crate::r#match::MatchResult;
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::utils::random::engine as rng_engine;
use crate::{Player, Staff, TeamInfo, TeamType};
use chrono::{Datelike, Duration, NaiveDateTime};
use rayon::prelude::*;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::time::Duration as StdDuration;

/// Lightweight country info for nationality lookups.
/// Covers ALL countries (not just simulation participants).
#[derive(Clone, Debug)]
pub struct CountryInfo {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
}

/// Upper bound on one Phase-B AI batch. Long enough for a cross-continent
/// monthly run with a slow provider; short enough that a hung service
/// still yields within a minute and the sim keeps ticking.
const AI_BATCH_TIMEOUT: StdDuration = StdDuration::from_secs(60);

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &'static str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if payload.downcast_ref::<String>().is_some() {
        "<String panic>"
    } else {
        "<non-string panic>"
    }
}

pub struct FootballSimulator;

impl FootballSimulator {
    pub async fn simulate(data: &mut SimulatorData) -> SimulationResult {
        let mut result = SimulationResult::new();

        let current_date = data.date;

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
        //
        // A panic inside one continent must not kill the whole tick — a
        // single buggy state machine or malformed save row would otherwise
        // unwind the Rayon pool and dump the player's save. We catch,
        // log, and substitute an empty result so the surviving continents
        // still advance.
        let results: Vec<ContinentResult> = data
            .continents
            .par_iter_mut()
            .map(|continent| {
                let cid = continent.id;
                let name = continent.name.clone();
                let ctx_ref = &ctx;
                panic::catch_unwind(AssertUnwindSafe(|| {
                    continent.simulate(ctx_ref.with_continent(cid))
                }))
                    .unwrap_or_else(|payload| {
                        let msg = panic_message(&payload);
                        log::error!(
                        "continent {} ({}) panicked during simulate: {}. tick continues with empty result.",
                        cid, name, msg
                    );
                        ContinentResult::new(cid, Vec::new(), Vec::new())
                    })
            })
            .collect();

        // Phase B: collect and batch-execute all AI requests. Guarded by
        // a timeout so a hung upstream provider can't stall the whole
        // simulation forever — on timeout, responses are dropped and the
        // tick advances without applying AI decisions.
        let all_requests = ctx.ai.drain();
        let ai_count = all_requests.len();
        if !all_requests.is_empty() {
            let fut = AiBatchProcessor::execute(all_requests);
            match tokio::time::timeout(AI_BATCH_TIMEOUT, fut).await {
                Ok(completed) => apply_ai_responses(completed, data),
                Err(_) => log::error!(
                    "AI batch timed out after {:?} ({} requests dropped), tick continues",
                    AI_BATCH_TIMEOUT, ai_count
                ),
            }
        }

        // Phase C: process the collected results against post-AI data
        for continent_result in results {
            continent_result.process(data, &mut result);
        }

        // Phase D: world-level manager market.
        //
        // Runs once per day after every continent's BoardResult.process has
        // applied sackings (so freshly-vacated seats and freshly-pooled
        // staff are visible in this same tick). Five steps:
        //   1. Sweep naturally-expired non-manager contracts into the pool.
        //   2. Age the pool (decay satisfaction, retire elderly coaches).
        //   3. Refresh shortlists for clubs in active manager search.
        //      Combined builder pulls from the free-agent pool AND
        //      enumerates poach-candidates at smaller clubs.
        //   4. Initiate fresh approaches for top employed candidates
        //      that don't already have a pursuit in flight.
        //   5. Advance every in-flight approach by one state — sources
        //      respond, candidates accept terms, signings finalize
        //      with their cascade source-club search.
        let today = data.date.date();
        free_pool::harvest_expired_staff(data, today);
        free_pool::tick_free_agent_staff_pool(&mut data.free_agent_staff, today);
        manager_market::refresh_shortlists(data);
        manager_market::initiate_approaches(data);
        manager_market::tick_approaches(data);

        // Global competitions (Champions League, World Cup, etc.)
        GlobalCompetitionSimulator::simulate(data);

        // Refresh player indexes only if a transfer actually moved a player
        // between clubs today. Walking the world every day is wasteful.

        if data.dirty_player_index {
            if let Some(mut indexes) = data.indexes.take() {
                indexes.refresh_player_indexes(data);
                data.indexes = Some(indexes);
            }
            data.dirty_player_index = false;
        }

        // Seed history for any players created today that haven't been seeded
        // (youth intake, regens, new clubs) — catches them within one tick.
        data.seed_missing_player_histories();

        // Once a month, prune the global match store to its retention window.
        // Cheap — BTreeMap range walk over evicted dates only.
        if current_date.day() == 1 {
            data.match_store.trim(current_date.date());
        }

        data.next_date();

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

    pub free_agents: Vec<Player>,

    /// Coaches/managers/staff between jobs. Populated on sacking and on
    /// natural contract expiry; drained when the manager market signs
    /// a candidate. Globally scoped so a Premier League club can hire
    /// a sacked Bundesliga manager without per-country plumbing.
    pub free_agent_staff: Vec<Staff>,

    /// In-flight approaches by clubs pursuing employed managers at
    /// other clubs (slice C — poaching). Each entry is one
    /// requesting-club ↔ candidate ↔ source-club triplet that
    /// progresses through `ApproachState` over ~5 daily ticks before
    /// either resolving in a signing (with cascade) or being rejected.
    pub pending_manager_approaches:
        Vec<crate::club::board::manager_market::ManagerApproach>,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,

    /// All countries by id (for nationality lookups — includes countries without active leagues)
    pub country_info: HashMap<u32, CountryInfo>,

    /// Global match result storage — all match types (league, cup, national team) write here
    pub match_store: MatchStorage,
}

impl SimulatorData {
    /// Build a SimulatorData with the deterministic sim RNG pinned to `seed`.
    /// Passing a non-zero seed makes the util-layer RNG stream reproducible
    /// per worker thread; Rayon scheduling still reorders draws across
    /// threads, so this is a debugging aid, not a replay tool.
    pub fn new_seeded(
        date: NaiveDateTime,
        continents: Vec<Continent>,
        global_competitions: GlobalCompetitions,
        seed: u64,
    ) -> Self {
        rng_engine::set_seed(seed);
        Self::new(date, continents, global_competitions)
    }

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
            free_agents: Vec::new(),
            free_agent_staff: Vec::new(),
            pending_manager_approaches: Vec::new(),
            watchlist: Vec::new(),
            global_competitions,
            country_info,
            match_store: MatchStorage::new(),
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
                // Pre-compute league (name, slug) lookup once per country.
                // Before: per-club closure did a linear scan of leagues,
                // costing ~N_clubs × N_leagues per country on affected days.
                let league_info: HashMap<u32, (String, String)> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
                    .collect();
                let lookup = |league_id: u32| -> (&str, &str) {
                    league_info
                        .get(&league_id)
                        .map(|(n, s)| (n.as_str(), s.as_str()))
                        .unwrap_or(("", ""))
                };

                for club in &mut country.clubs {
                    // In the `only_empty` path (steady state) most teams have
                    // zero players needing seeding — skip the whole club as
                    // cheaply as possible before we pay any allocation cost.
                    if only_empty
                        && !club.teams.iter().any(|t| {
                        t.players.iter().any(|p| p.statistics_history.needs_current_season_seed())
                    })
                    {
                        continue;
                    }

                    // Resolve the main team's identity once per club.
                    let main_team = club.teams.main();
                    let main_name = main_team.map(|t| t.name.clone());
                    let main_slug = main_team.map(|t| t.slug.clone());
                    let main_reputation = main_team.map(|t| t.reputation.world);
                    let (main_league_name, main_league_slug) = main_team
                        .and_then(|t| t.league_id)
                        .map(|lid| {
                            let (n, s) = lookup(lid);
                            (n.to_owned(), s.to_owned())
                        })
                        .unwrap_or_default();

                    for team in club.teams.iter_mut() {
                        // Cheap per-team scan — skip if no seeding needed.
                        if only_empty
                            && !team.players.iter().any(|p| p.statistics_history.needs_current_season_seed())
                        {
                            continue;
                        }

                        let team_info = if team.team_type == TeamType::Main || main_name.is_none() {
                            TeamInfo {
                                name: team.name.clone(),
                                slug: team.slug.clone(),
                                reputation: team.reputation.world,
                                league_name: main_league_name.clone(),
                                league_slug: main_league_slug.clone(),
                            }
                        } else {
                            TeamInfo {
                                name: main_name.clone().unwrap_or_default(),
                                slug: main_slug.clone().unwrap_or_default(),
                                reputation: main_reputation.unwrap_or(0),
                                league_name: main_league_name.clone(),
                                league_slug: main_league_slug.clone(),
                            }
                        };

                        for player in &mut team.players.players {
                            if only_empty && !player.statistics_history.needs_current_season_seed() {
                                continue;
                            }
                            let is_loan = player.is_on_loan();
                            player.statistics_history.seed_initial_team(&team_info, date, is_loan);
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
