use crate::PlayerFieldPositionGroup;
use crate::PlayerSquadStatus;
use crate::ai::{AiBatchProcessor, PendingAiRequest};
use crate::club::ai::apply_ai_responses;
use crate::club::board::manager_market;
use crate::club::player::calculators::WageCalculator;
use crate::competitions::GlobalCompetitions;
use crate::competitions::simulation::GlobalCompetitionSimulator;
use crate::config::SimulatorConfig;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::national::world as national_world;
use crate::continent::{Continent, ContinentResult};
use crate::country::result::transfers::free_agent_market_calc::FreeAgentMarketCalculator;
use crate::country::result::transfers::{GlobalFreeAgentSummary, snapshot_global_free_agents};
use crate::league::awards::{
    AwardAggregator, CandidateAggregate, MonthlyAwardSelector, MonthlyAwardsSnapshot,
    MonthlyPlayerAward, MonthlyStatLeader, SeasonAwardsSnapshot, TeamOfTheWeekAward,
    TeamOfTheWeekSelector, TeamOfTheWeekSlot, TeamOfTheYearAward, WeeklyAggregate,
    YOUNG_WEEKLY_MAX_AGE,
};
use crate::league::player_of_week::{PlayerOfTheWeekAward, PlayerOfTheWeekSelector};
use crate::league::{LeagueTable, MatchStorage};
use crate::r#match::MatchResult;
use crate::perf::{PerfCounters, PerfPhase, TickEndContext};
use crate::shared::SimulatorDataIndexes;
use crate::transfers::TransferPool;
use crate::transfers::pipeline::{PipelineProcessor, PlayerSummary};
use crate::utils::DateUtils;
use crate::utils::IntegerUtils;
use crate::utils::random::engine as rng_engine;
use crate::{
    AwardReputationInput, AwardReputationKind, HappinessEventType, Person, Player,
    RecognitionEventContext, RecognitionEventKind, Staff, TeamInfo,
};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, Weekday};
use rayon::prelude::*;
use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};

/// Lightweight country info for nationality lookups.
/// Covers ALL countries (not just simulation participants).
#[derive(Clone, Debug)]
pub struct CountryInfo {
    pub id: u32,
    pub code: String,
    pub slug: String,
    pub name: String,
    /// Continent the country sits on. Carried here so the region-prestige
    /// gate used by the loan market, scouting, and personal-terms
    /// negotiation can resolve a `ScoutingRegion` even for nationalities
    /// whose country has no active leagues in this save.
    pub continent_id: u32,
    /// Football reputation (0..10000). Mirrors the same field on `Country`
    /// so the country-reputation realism gate keeps working when the
    /// nationality's leagues aren't loaded — without this it falls back to
    /// `0` and an Argentinian free agent slips through to a Mali buyer.
    pub reputation: u16,
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &'static str {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        s
    } else if payload.downcast_ref::<String>().is_some() {
        "<String panic>"
    } else {
        "<non-string panic>"
    }
}

/// Cumulative count of continent panics swallowed by the simulator. The
/// `simulate` loop catches a panicking continent and substitutes an empty
/// result so the rest of the world keeps ticking — this counter exposes
/// that silent failure to operators and tests. Read from anywhere via
/// `panicked_continents_total()`.
static PANICKED_CONTINENTS: AtomicU64 = AtomicU64::new(0);

/// Total continent panics swallowed since process start.
pub fn panicked_continents_total() -> u64 {
    PANICKED_CONTINENTS.load(Ordering::Relaxed)
}

pub struct FootballSimulator;

impl FootballSimulator {
    /// Tick the simulator one day with default tunables. Use `simulate_with`
    /// to plumb a `SimulatorConfig` (per-save overrides, faster timeouts in
    /// tests, etc.).
    pub async fn simulate(data: &mut SimulatorData) -> SimulationResult {
        Self::simulate_with(data, &SimulatorConfig::default()).await
    }

    pub async fn simulate_with(
        data: &mut SimulatorData,
        config: &SimulatorConfig,
    ) -> SimulationResult {
        let perf = PerfCounters::instance();
        perf.begin_tick();

        let mut result = SimulationResult::new();

        let current_date = data.date;

        let ctx = GlobalContext::new(SimulationContext::new(data.date));

        // National-team call-ups run at the world level so a player's
        // nationality and their club's continent can differ. Must
        // happen BEFORE the world-level national-competition phase —
        // those matches need a populated squad with world visibility.
        {
            let _g = perf.scope(PerfPhase::WorldCallups);
            data.process_world_national_team_callups();
        }

        // National-team competition matches simulate at the world level
        // so squads can include foreign-based players and post-match
        // stats updates fan out across every continent. Lifted out of
        // the parallel continent phase because squad construction needs
        // read access to clubs in *every* continent.
        let national_match_results = {
            let _g = perf.scope(PerfPhase::WorldNationalMatches);
            national_world::simulate_world_national_competitions(
                &mut data.continents,
                current_date.date(),
            )
        };
        for match_result in &national_match_results {
            data.match_store
                .push(match_result.clone(), current_date.date());
        }
        result.match_results.extend(national_match_results);

        // Phase ordering note:
        // A simulates continents and surfaces AI requests inside each
        // ContinentResult — no shared collector, no lock contention. B
        // drains those requests, batch-executes them, and applies
        // responses against the freshly-mutated data. C then drains the
        // rest of each ContinentResult. Requests carry stable IDs
        // (club_id, player_id, …) so Phase B mutations (contracts,
        // morale, etc.) are safely visible to Phase C.

        // Phase A: simulate all continents in parallel. Each call mutates
        // its own continent and stages AI requests on its returned
        // `ContinentResult.pending_ai_requests` — no shared state across
        // workers.
        //
        // A panic inside one continent must not kill the whole tick — a
        // single buggy state machine or malformed save row would otherwise
        // unwind the Rayon pool and dump the player's save. `AssertUnwindSafe`
        // is sound here because the closure mutates only its own continent
        // (no shared `&mut` state) and doesn't hold any locks; the Rayon
        // worker doesn't carry poisoned state across iterations. Panic is
        // surfaced via the `PANICKED_CONTINENTS` counter and a structured
        // log line; surviving continents still advance. Per-tick count
        // is recovered as the delta on the atomic since map closures
        // running in parallel can't share a `&mut u32`.
        let panicks_before = PANICKED_CONTINENTS.load(Ordering::Relaxed);
        let mut results: Vec<ContinentResult> = {
            let _g = perf.scope(PerfPhase::ParallelContinents);
            data
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
                        PANICKED_CONTINENTS.fetch_add(1, Ordering::Relaxed);
                        let msg = panic_message(&payload);
                        log::error!(
                            "event=continent_panic continent_id={} continent_name={:?} message={:?} tick_action=continue_with_empty_result",
                            cid, name, msg
                        );
                        ContinentResult::new(cid, Vec::new(), Vec::new())
                    })
                })
                .collect()
        };
        result.panicked_continents =
            (PANICKED_CONTINENTS.load(Ordering::Relaxed) - panicks_before) as u32;

        // Phase B: drain AI requests staged on each ContinentResult and
        // batch-execute them. Lock-free — every request travelled up the
        // result chain owned by exactly one worker. The tick waits for
        // the batch to finish — no timeout, no dropped responses.
        let mut all_requests: Vec<PendingAiRequest> = Vec::new();
        for cr in &mut results {
            if !cr.pending_ai_requests.is_empty() {
                all_requests.append(&mut cr.pending_ai_requests);
            }
        }
        if !all_requests.is_empty() {
            perf.record_ai_batch_active();
            let _g = perf.scope(PerfPhase::AiBatch);
            let completed = AiBatchProcessor::execute(all_requests).await;
            apply_ai_responses(completed, data);
        }

        // Phase C: process the collected results against post-AI data
        {
            let _g = perf.scope(PerfPhase::ResultProcessing);

            // Build the world-wide foreign-player pool ONCE for this tick.
            // `simulate_transfer_market` runs per country and previously
            // rebuilt this pool on every call by walking every other
            // country's players (O(N²) in countries × O(P) per country).
            // The seller context is per-source-country, so a global
            // snapshot is correct as long as we filter by buyer country
            // at the read site. Cache lives on `data` so deep callers
            // pick it up without a signature change.
            let pool_date = data.date.date();
            let world_pool: Vec<PlayerSummary> = data
                .continents
                .iter()
                .flat_map(|cont| &cont.countries)
                .flat_map(|c| PipelineProcessor::collect_player_pool(c, pool_date))
                .collect();
            data.daily_world_player_pool = Some(world_pool);

            // Same trick for the global free-agent snapshot. The
            // function mutates `data.free_agents` (idempotent on
            // repeat with the same date) and previously fired once
            // per country. Building once here drops it from N calls
            // to 1.
            let global_fa_snapshot: Vec<GlobalFreeAgentSummary> =
                snapshot_global_free_agents(data, pool_date);
            data.daily_global_free_agents = Some(global_fa_snapshot);

            for continent_result in results {
                continent_result.process(data, &mut result);
            }

            // Drop the caches so the next tick rebuilds — players move
            // between clubs/countries and free-agent state evolves; the
            // per-tick snapshot would otherwise grow stale.
            data.daily_world_player_pool = None;
            data.daily_global_free_agents = None;
        }

        // Phase D: world-level manager market. Order is load-bearing —
        // see `ManagerMarketTick::run` for the dependency rationale.
        let today = data.date.date();
        {
            let _g = perf.scope(PerfPhase::ManagerMarket);
            manager_market::ManagerMarketTick::run(data, today);
        }

        // Phase D2: parent-side loan wage settlement. Per-club monthly
        // finance runs inside Phase A and bills the borrower for the
        // loan contract; the parent club still owes the residual share
        // of its primary contract for the duration of the loan. Done
        // here at the world level because parent and borrower may live
        // in different countries — a per-country pass can't see them
        // both.
        if today.day() == 1 {
            settle_parent_residual_loan_wages(data);
            // Long-unemployed free agents eventually hang up the boots.
            // Monthly check, gated internally on `free_since` >= 12mo.
            data.process_free_agent_retirements(today);
        }

        // Global competitions (Champions League, World Cup, etc.)
        {
            let _g = perf.scope(PerfPhase::GlobalCompetitions);
            GlobalCompetitionSimulator::simulate(data);
        }

        // Release Int statuses AFTER all matches (continent + global) —
        // a tournament final on the release date should be played
        // before the squad's flags are cleared.
        let dirty_before_rebuild;
        {
            let _g = perf.scope(PerfPhase::Cleanup);
            data.process_world_national_team_release();

            // Move any player whose contract was cleared this tick (positional
            // surplus, free-transfer release, contract expiry) off their old
            // team's roster and into the global free-agent pool, so the player
            // page header and contract panel agree.
            data.sweep_released_to_free_agents();

            // Refresh player indexes only if a transfer actually moved a player
            // between clubs today. Walking the world every day is wasteful.
            dirty_before_rebuild = data.dirty_player_index;
            data.rebuild_indexes_if_dirty();
            if dirty_before_rebuild {
                perf.mark_dirty_index_rebuild();
            }

            // Seed history for any players created today that haven't been seeded
            // (youth intake, regens, new clubs) — catches them within one tick.
            data.seed_missing_player_histories();

            // Periodic prune of the global match store. Cadence lives on the
            // config (default: first of every month). Cheap — BTreeMap range
            // walk over evicted dates only.
            if config.is_trim_day(current_date.date()) {
                data.match_store.trim(current_date.date());
            }
        }

        // Order: largest weekly award first so the centralised
        // award-reputation pipeline can dampen the smaller award when
        // both go to the same player. Young POW fires before senior
        // POW because the breakthrough-amplified base is larger;
        // Team selections are dampened against either weekly winner.
        //
        // The four Monday tickers all need per-league weekly aggregates.
        // Build them once (in parallel across leagues) and share the
        // `MondayAwardCache` across all four — the previous design had
        // each tick re-aggregate the same week's matches independently.
        let today = data.date.date();
        {
            let _g = perf.scope(PerfPhase::Awards);
            if today.weekday() == Weekday::Mon {
                let week_end = today;
                let week_start = today - Duration::days(7);
                let cache = MondayAwardCache::build(data, week_start, week_end);
                // Pick each league's Young Player of the Week (age ≤ 20).
                YoungWeeklyAwardsTick::run(data, &cache);
                // Pick each league's Player of the Week. Runs every Monday
                // after the matchday pipeline has flushed last week's results
                // into each league's MatchStorage.
                WeeklyAwardsTick::run(data, &cache);
                // Young Team of the Week (age ≤ 20). Same window as Team of
                // the Week.
                YoungTeamOfTheWeekTick::run(data, &cache);
                // Team of the Week — one XI per league, every Monday.
                TeamOfTheWeekTick::run(data, &cache);
            }
            // Monthly awards — first day of each month, awarding the previous
            // calendar month.
            MonthlyAwardsTick::run(data);
            // Drain any league-side pending season-awards snapshots and emit
            // the player events while stats are still meaningful.
            SeasonAwardsTick::run(data);
            // Calendar-year XI per league — runs once on December 31.
            TeamOfTheYearTick::run(data);
            // World player of the year — runs once per year. Builds a global
            // ranking from per-continent rankings so a top performer in any
            // league can win.
            WorldPlayerOfYearTick::run(data);
        }

        data.next_date();

        let workload = data.workload_counts();
        perf.end_tick(TickEndContext {
            countries: workload.countries,
            leagues: workload.leagues,
            clubs: workload.clubs,
            players: workload.players,
            match_results_written: result.match_results.len() as u64,
            panicked_continents: result.panicked_continents,
            recording_mode: crate::is_match_recordings_mode(),
        });

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
    pub pending_manager_approaches: Vec<crate::club::board::manager_market::ManagerApproach>,

    pub watchlist: Vec<u32>,

    pub global_competitions: GlobalCompetitions,

    /// All countries by id (for nationality lookups — includes countries without active leagues)
    pub country_info: HashMap<u32, CountryInfo>,

    /// Global match result storage — all match types (league, cup, national team) write here
    pub match_store: MatchStorage,

    /// Per-tick scratch cache: every non-loaned player in the world,
    /// summarised once at the top of Phase C so per-country transfer
    /// markets reuse the snapshot instead of rebuilding it per call.
    /// Reset (`= None`) at the end of each `simulate_with` tick;
    /// readers fall back to a local rebuild when the cache is `None`
    /// so test paths and one-off callers still work.
    pub daily_world_player_pool: Option<Vec<PlayerSummary>>,

    /// Per-tick scratch cache: snapshot of every globally-pooled free
    /// agent. Same lifecycle as `daily_world_player_pool` —
    /// `simulate_transfer_market` would otherwise call
    /// `snapshot_global_free_agents` per country, which mutates each
    /// player's `free_agent_state` (idempotent on repeat with the same
    /// date) and walks every free agent. Crate-private because the
    /// snapshot type is internal to the country/result module.
    pub(crate) daily_global_free_agents: Option<Vec<GlobalFreeAgentSummary>>,
}

impl SimulatorData {
    /// Build a SimulatorData with the deterministic sim RNG pinned to `seed`.
    /// Passing a non-zero seed makes the util-layer RNG stream reproducible
    /// per worker thread; Rayon scheduling still reorders draws across
    /// threads, so this is a debugging aid, not a replay tool.
    ///
    /// **Note: the seed is process-global state.** `set_seed` writes to
    /// the RNG engine's static; building two `SimulatorData` back-to-back
    /// means the second silently inherits whatever seed the first left
    /// behind unless this function (or `set_seed`) is called again.
    /// Don't rely on this constructor to fully isolate two simulators
    /// running in the same process.
    pub fn new_seeded(
        date: NaiveDateTime,
        continents: Vec<Continent>,
        global_competitions: GlobalCompetitions,
        seed: u64,
    ) -> Self {
        rng_engine::set_seed(seed);
        Self::new(date, continents, global_competitions)
    }

    /// Build a SimulatorData populated from `continents`.
    ///
    /// **`country_info` lifecycle:** the constructor seeds the nationality
    /// lookup map only with countries that participate in the simulation
    /// (i.e. countries whose continents are passed in). Some nationalities
    /// belong to countries that have no active leagues — those need to be
    /// added explicitly via [`add_country_info`] by the database loader
    /// before the first `simulate()` call. A nationality lookup that misses
    /// returns `None` silently, so a forgotten generator step manifests as
    /// blank flags / empty country names in the UI rather than a panic.
    pub fn new(
        date: NaiveDateTime,
        continents: Vec<Continent>,
        global_competitions: GlobalCompetitions,
    ) -> Self {
        // Build country_info from simulation participants
        let country_info: HashMap<u32, CountryInfo> = continents
            .iter()
            .flat_map(|cont| &cont.countries)
            .map(|c| {
                (
                    c.id,
                    CountryInfo {
                        id: c.id,
                        code: c.code.clone(),
                        slug: c.slug.clone(),
                        name: c.name.clone(),
                        continent_id: c.continent_id,
                        reputation: c.reputation,
                    },
                )
            })
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
            daily_world_player_pool: None,
            daily_global_free_agents: None,
        };

        let mut indexes = SimulatorDataIndexes::new();

        indexes.refresh(&data);

        data.indexes = Some(indexes);

        data.init_league_tables();
        data.seed_player_histories();
        data.seed_player_nationality_continents();

        data
    }

    /// Populate `Player.nationality_continent_id` from `country_info` for
    /// every player on every roster + retired + national-team + free-agent
    /// pool. Called once at construction time after `country_info` is
    /// populated. Cheap parallel pass.
    pub fn seed_player_nationality_continents(&mut self) {
        let lookup: std::collections::HashMap<u32, u32> = self
            .country_info
            .iter()
            .map(|(k, v)| (*k, v.continent_id))
            .collect();
        if lookup.is_empty() {
            return;
        }
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                for club in &mut country.clubs {
                    for team in club.teams.iter_mut() {
                        for player in &mut team.players.players {
                            if player.nationality_continent_id == 0 {
                                if let Some(cid) = lookup.get(&player.country_id) {
                                    player.nationality_continent_id = *cid;
                                }
                            }
                        }
                    }
                }
                for player in &mut country.retired_players {
                    if player.nationality_continent_id == 0 {
                        if let Some(cid) = lookup.get(&player.country_id) {
                            player.nationality_continent_id = *cid;
                        }
                    }
                }
                for player in &mut country.national_team.generated_squad {
                    if player.nationality_continent_id == 0 {
                        if let Some(cid) = lookup.get(&player.country_id) {
                            player.nationality_continent_id = *cid;
                        }
                    }
                }
            });
        for player in &mut self.free_agents {
            if player.nationality_continent_id == 0 {
                if let Some(cid) = lookup.get(&player.country_id) {
                    player.nationality_continent_id = *cid;
                }
            }
        }
    }

    /// Register country info for countries that may not have active leagues in the simulation.
    /// Called by the database generator to ensure nationality lookups always succeed.
    pub fn add_country_info(
        &mut self,
        id: u32,
        code: String,
        slug: String,
        name: String,
        continent_id: u32,
        reputation: u16,
    ) {
        self.country_info.entry(id).or_insert(CountryInfo {
            id,
            code,
            slug,
            name,
            continent_id,
            reputation,
        });
    }

    /// Walk every player slot in the simulator and bump the procedural id
    /// sequence past the highest id seen. The single source of truth for
    /// future id allocation — call this after world generation (and after
    /// any future save-load path) so runtime academy intake / U18 fallback
    /// can never collide with an id that already exists in the world.
    /// Cheap: a single pass over all rosters; only runs at startup.
    pub fn seed_player_id_sequence(&self) {
        let mut max_id: u32 = 0;
        for continent in &self.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        for player in &team.players.players {
                            if player.id > max_id {
                                max_id = player.id;
                            }
                        }
                    }
                }
                for player in &country.retired_players {
                    if player.id > max_id {
                        max_id = player.id;
                    }
                }
                for player in &country.national_team.generated_squad {
                    if player.id > max_id {
                        max_id = player.id;
                    }
                }
            }
        }
        for player in &self.free_agents {
            if player.id > max_id {
                max_id = player.id;
            }
        }
        crate::seed_core_player_id_sequence(max_id);
    }

    /// Remove a country from the nationality lookup map.
    pub fn remove_country_info(&mut self, id: u32) {
        self.country_info.remove(&id);
    }

    /// Initial population of league tables at construction time.
    /// Per-season rebuilds happen inside `League::simulate` when a new
    /// schedule is generated. The skip-if-non-empty guard below is
    /// therefore intentional: it only prevents the initial seed from
    /// clobbering an already-populated table.
    fn init_league_tables(&mut self) {
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let clubs = &country.clubs;
                for league in &mut country.leagues.leagues {
                    if !league.table.rows.is_empty() {
                        continue;
                    }
                    let team_ids = team_ids_for_league(clubs, league.id);
                    if !team_ids.is_empty() {
                        league.table = LeagueTable::new(&team_ids);
                    }
                }
            });
    }

    /// Seed statistics history for every player. Called once at
    /// construction time — touches every player unconditionally.
    fn seed_player_histories(&mut self) {
        let date = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let league_lookup = build_league_lookup(country);
                for club in &mut country.clubs {
                    let club_ctx = ClubSeedingContext::resolve(club, &league_lookup);
                    for team in club.teams.iter_mut() {
                        let team_info = club_ctx.team_info_for(team);
                        for player in &mut team.players.players {
                            let is_loan = player.is_on_loan();
                            player
                                .statistics_history
                                .seed_initial_team(&team_info, date, is_loan);
                        }
                    }
                }
            });
    }

    /// Seed any players whose history is still empty — catches youth intake,
    /// regens, and newly-generated clubs within one simulated tick.
    /// Skip-fast at club AND team level so the steady-state cost is close
    /// to zero when nothing needs seeding.
    pub fn seed_missing_player_histories(&mut self) {
        let date = self.date.date();
        self.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let league_lookup = build_league_lookup(country);
                for club in &mut country.clubs {
                    if !club_has_players_needing_seed(club) {
                        continue;
                    }
                    let club_ctx = ClubSeedingContext::resolve(club, &league_lookup);
                    for team in club.teams.iter_mut() {
                        if !team_has_players_needing_seed(team) {
                            continue;
                        }
                        let team_info = club_ctx.team_info_for(team);
                        for player in &mut team.players.players {
                            if !player.statistics_history.needs_current_season_seed() {
                                continue;
                            }
                            let is_loan = player.is_on_loan();
                            player
                                .statistics_history
                                .seed_initial_team(&team_info, date, is_loan);
                        }
                    }
                }
            });
    }

    /// Move every team-attached player whose main-club contract is `None`
    /// onto the global `free_agents` pool. Several pipelines (positional
    /// surplus, unresolved-salary "free transfer", contract expiry) clear
    /// the contract in place; without this sweep the player lingers on the
    /// roster as a "free agent on a team," which the player page renders
    /// inconsistently — the header reads the team name while the contract
    /// panel reads "Free Agent."
    ///
    /// Each move is logged as a `CompletedTransfer` (zero fee, `Free`
    /// type) on the losing club's country, so the transfer history page
    /// reflects the departure. Reason is derived from the player's
    /// status: `Frt` set means the club explicitly released early
    /// (mutual / surplus / unresolved-salary path); otherwise the
    /// contract simply expired.
    ///
    /// Loanees are skipped (their `contract` is the parent-club contract
    /// and stays `Some` during the loan), as are retired players (already
    /// removed from team rosters by the retirement pipeline). Sets
    /// `dirty_player_index` so the next index rebuild picks up the moves.
    pub fn sweep_released_to_free_agents(&mut self) {
        use crate::PlayerStatusType;
        use crate::club::player::transfer::ReleaseContext;
        use crate::shared::{Currency, CurrencyValue};
        use crate::transfers::{CompletedTransfer, TransferType};

        let date = self.date.date();
        let released: Vec<Player> = self
            .continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .flat_map_iter(|country| {
                // League reputation is needed by `on_release` so the
                // player carries an accurate market-state snapshot into
                // the free-agent pool. Pre-collect once per country —
                // immutable read before the mutable club iteration
                // takes the borrow.
                let league_reputations: std::collections::HashMap<u32, u16> = country
                    .leagues
                    .leagues
                    .iter()
                    .map(|l| (l.id, l.reputation))
                    .collect();
                let country_id = country.id;
                let country_reputation = country.reputation;
                let mut released_in_country: Vec<Player> = Vec::new();
                let mut new_history: Vec<CompletedTransfer> = Vec::new();
                for club in &mut country.clubs {
                    let club_id = club.id;
                    for team in &mut club.teams.teams {
                        let team_id = team.id;
                        let team_name = team.name.clone();
                        let team_reputation_world = team.reputation.world;
                        let team_league_reputation = team
                            .league_id
                            .and_then(|lid| league_reputations.get(&lid).copied())
                            .unwrap_or(country_reputation);
                        let candidates: Vec<(u32, String, bool)> = team
                            .players
                            .players
                            .iter()
                            .filter(|p| p.contract.is_none() && !p.is_on_loan() && !p.retired)
                            .map(|p| {
                                let was_released_early =
                                    p.statuses.get().contains(&PlayerStatusType::Frt);
                                (p.id, p.full_name.to_string(), was_released_early)
                            })
                            .collect();
                        for (id, player_name, released_early) in candidates {
                            if let Some(mut p) = team.players.take_player(&id) {
                                let reason = if released_early {
                                    "dec_reason_released_free".to_string()
                                } else {
                                    "dec_reason_contract_expired".to_string()
                                };
                                new_history.push(
                                    CompletedTransfer::new(
                                        id,
                                        player_name,
                                        club_id,
                                        team_id,
                                        team_name.clone(),
                                        0,
                                        "Free Agent".to_string(),
                                        date,
                                        CurrencyValue::new(0.0, Currency::Usd),
                                        TransferType::Free,
                                    )
                                    .with_reason(reason),
                                );
                                // Stamp the player's market-state
                                // snapshot at the moment they enter the
                                // pool. `last_salary` is unrecoverable
                                // here (the contract was already cleared
                                // upstream), so seed from the wage
                                // calculator using the team / league
                                // tiers as a faithful replacement.
                                let last_squad_status = PlayerSquadStatus::FirstTeamSquadRotation;
                                let club_score =
                                    (team_reputation_world as f32 / 10_000.0).clamp(0.0, 1.0);
                                let last_salary = WageCalculator::expected_annual_wage(
                                    &p,
                                    p.age(date),
                                    club_score,
                                    team_league_reputation,
                                );
                                if p.free_agent_state().is_none() {
                                    p.enter_free_agent_market(ReleaseContext {
                                        date,
                                        last_club_id: Some(club_id),
                                        last_country_id: Some(country_id),
                                        last_country_reputation: country_reputation,
                                        last_league_reputation: team_league_reputation,
                                        last_club_reputation_score: club_score,
                                        last_salary,
                                        last_squad_status,
                                    });
                                }
                                released_in_country.push(p);
                            }
                        }
                    }
                }
                country.transfer_market.transfer_history.extend(new_history);
                released_in_country
            })
            .collect();
        if !released.is_empty() {
            self.dirty_player_index = true;
            self.free_agents.extend(released);
        }
    }

    /// Monthly retirement pass over the global free-agent pool. Anyone
    /// 12+ months without a club rolls retirement at a probability that
    /// climbs with age, low quality, and time spent unemployed; high
    /// world-rep players resist longer (they're still names, clubs come
    /// looking).
    ///
    /// Gated by the caller on `today.day() == 1`. The internal gate on
    /// `free_since` ≥ 12 months means a fresh database free agent
    /// (seeded `free_since = today - 30d`) is automatically skipped.
    pub fn process_free_agent_retirements(&mut self, date: NaiveDate) {
        use crate::PlayerStatusType;

        let mut to_retire: Vec<usize> = Vec::new();
        for (idx, player) in self.free_agents.iter().enumerate() {
            let Some(state) = player.free_agent_state() else {
                continue;
            };
            let days_free = (date - state.free_since).num_days();
            if days_free < 365 {
                continue;
            }
            let months_after_12 = ((days_free - 365) / 30).max(0) as u32;
            let prob = FreeAgentMarketCalculator::retirement_probability_per_month(
                months_after_12,
                player.age(date),
                player.player_attributes.current_ability,
                player.player_attributes.world_reputation,
            );
            if prob <= 0.0 {
                continue;
            }
            let roll = IntegerUtils::random(1, 1000) as f32 / 1000.0;
            if roll < prob {
                to_retire.push(idx);
            }
        }

        // Reverse order so swap_remove against earlier indexes doesn't
        // disturb later ones.
        to_retire.sort_unstable_by(|a, b| b.cmp(a));
        for idx in to_retire {
            let mut player = self.free_agents.swap_remove(idx);
            player.statuses.add(date, PlayerStatusType::Ret);
            player.contract = None;
            player.retired = true;
            let country_id = player.country_id;
            if let Some(country) = self.country_mut(country_id) {
                country.retired_players.push(player);
            }
            // Else: nationality country isn't loaded — drop silently.
            // The player is gone from the pool either way.
        }
    }

    pub fn next_date(&mut self) {
        self.date += Duration::days(1);
    }

    /// Walk the world once to count countries, leagues, clubs and
    /// players. Used by the perf dashboard at end-of-tick — single
    /// linear pass, no allocation.
    pub fn workload_counts(&self) -> WorldWorkloadCounts {
        let mut counts = WorldWorkloadCounts {
            countries: 0,
            leagues: 0,
            clubs: 0,
            players: 0,
        };
        for continent in &self.continents {
            for country in &continent.countries {
                counts.countries += 1;
                counts.leagues += country.leagues.leagues.len() as u64;
                counts.clubs += country.clubs.len() as u64;
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        counts.players += team.players.players.len() as u64;
                    }
                }
            }
        }
        counts
    }

    /// World-level national-team call-ups. Runs at the start of each
    /// break/tournament window, before any continent simulates, so
    /// candidate visibility spans the entire world — a Brazilian
    /// playing at a Spanish club is reachable from Brazil's selection
    /// pool without per-continent plumbing.
    pub fn process_world_national_team_callups(&mut self) {
        let date = self.date.date();
        let need_callups = crate::NationalTeam::is_break_start(date)
            || crate::NationalTeam::is_tournament_start(date);
        if !need_callups {
            return;
        }

        // Build a global candidate pool from every club in every country.
        let mut candidates_by_country = crate::NationalTeam::collect_all_candidates_by_country(
            self.continents.iter().flat_map(|c| c.countries.iter()),
            date,
        );

        // Country IDs across the whole world — used to draw friendly
        // opponents from any nation, not just same-continent.
        let country_ids: Vec<(u32, String)> = self
            .continents
            .iter()
            .flat_map(|c| c.countries.iter())
            .map(|c| (c.id, c.name.clone()))
            .collect();

        // Pre-distribute candidates per country so each rayon worker owns
        // its own slice — no shared HashMap, no lock. The serial drain
        // here is O(countries) and trivial next to the parallel
        // `call_up_squad` body.
        let work_items: Vec<_> = self
            .continents
            .iter_mut()
            .flat_map(|c| c.countries.iter_mut())
            .map(|country| {
                let candidates = candidates_by_country
                    .remove(&country.id)
                    .unwrap_or_default();
                (country, candidates)
            })
            .collect();

        work_items
            .into_par_iter()
            .for_each(|(country, candidates)| {
                country.national_team.country_name = country.name.clone();
                country.national_team.reputation = country.reputation;
                let cid = country.id;
                country
                    .national_team
                    .call_up_squad(candidates, date, cid, &country_ids);
            });

        // Apply Int status across every club in every continent.
        crate::NationalTeam::apply_callup_statuses_across_world(&mut self.continents, date);
    }

    /// World-level Int release. Runs after all matches (continent
    /// matches + global tournament matches) so a tournament final
    /// landing on a release date is played with squad statuses still
    /// attached. Squad data itself is preserved for the squad UI; only
    /// the per-player Int flag is cleared.
    pub fn process_world_national_team_release(&mut self) {
        let date = self.date.date();
        let need_release =
            crate::NationalTeam::is_break_end(date) || crate::NationalTeam::is_tournament_end(date);
        if !need_release {
            return;
        }
        crate::NationalTeam::release_callup_statuses_across_world(&mut self.continents);
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldWorkloadCounts {
    pub countries: u64,
    pub leagues: u64,
    pub clubs: u64,
    pub players: u64,
}

pub struct SimulationResult {
    pub match_results: Vec<MatchResult>,
    /// Number of continents whose `simulate` call panicked during this
    /// tick. Surfaces silent failures the orchestrator catches and
    /// substitutes empty results for. Sum across ticks via the
    /// process-global `panicked_continents_total()`.
    pub panicked_continents: u32,
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
            panicked_continents: 0,
        }
    }

    pub fn has_match_results(&self) -> bool {
        !self.match_results.is_empty()
    }
}

/// Walk every player in the world, find loaned-in players, and bill
/// the parent club for its residual share of the primary contract that
/// the borrower's loan contract didn't cover. Runs once per calendar
/// month from `simulate_with`.
///
/// Residual = `(parent_salary - loan_salary).max(0) / 12`. When
/// `loan_wage_contribution_pct` is recorded it implies the loan salary
/// is already a percentage of the parent salary, so the residual
/// arithmetic is correct without a separate pct path. Negative
/// residuals (borrower paying more than the parent contract — should
/// not happen in practice) are clamped to zero so we never accidentally
/// credit the parent.
fn settle_parent_residual_loan_wages(data: &mut SimulatorData) {
    // Pass 1 (read): collect (parent_club_id, monthly_residual) entries
    // so we don't hold borrows across the credit pass. The world-wide
    // walk parallelises across countries — every player is read-only
    // here, the merge into the HashMap happens serially below.
    let entries: Vec<(u32, i64)> = data
        .continents
        .par_iter()
        .flat_map(|c| c.countries.par_iter())
        .flat_map_iter(|country| {
            country.clubs.iter().flat_map(|club| {
                club.teams.teams.iter().flat_map(|team| {
                    team.players.players.iter().filter_map(|player| {
                        let loan = player.contract_loan.as_ref()?;
                        let parent_id = loan.loan_from_club_id?;
                        let parent_contract = player.contract.as_ref()?;
                        let parent_annual = parent_contract.salary;
                        let borrower_annual = loan.salary;
                        let residual_annual = parent_annual.saturating_sub(borrower_annual);
                        if residual_annual == 0 {
                            return None;
                        }
                        let monthly = (residual_annual / 12) as i64;
                        if monthly > 0 {
                            Some((parent_id, monthly))
                        } else {
                            None
                        }
                    })
                })
            })
        })
        .collect();

    let mut owed_by_parent: HashMap<u32, i64> = HashMap::new();
    for (parent_id, monthly) in entries {
        *owed_by_parent.entry(parent_id).or_insert(0) += monthly;
    }

    // Pass 2 (write): charge each parent club once.
    for (parent_id, amount) in owed_by_parent {
        if let Some(club) = data.club_mut(parent_id) {
            club.finance.balance.push_expense_player_wages(amount);
        }
    }
}

/// Per-league aggregates for a single Monday window, built once and
/// shared across all four weekly award ticks (Player of the Week, Young
/// Player of the Week, Team of the Week, Young Team of the Week). Each
/// tick used to walk every league's match storage and re-aggregate the
/// same window — four full passes per league per Monday. This caches
/// both the `WeeklyAggregate` (driver of Player of the Week) and the
/// `CandidateAggregate` (driver of Team of the Week) so the four ticks
/// reduce to a per-league lookup.
struct MondayAwardCache {
    weekly: HashMap<u32, HashMap<u32, WeeklyAggregate>>,
    candidate: HashMap<u32, HashMap<u32, CandidateAggregate>>,
}

impl MondayAwardCache {
    fn build(
        data: &SimulatorData,
        week_start: chrono::NaiveDate,
        week_end: chrono::NaiveDate,
    ) -> Self {
        let entries: Vec<(
            u32,
            HashMap<u32, WeeklyAggregate>,
            HashMap<u32, CandidateAggregate>,
        )> = data
            .continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter(|league| !league.friendly)
            .map(|league| {
                let weekly = PlayerOfTheWeekSelector::aggregate(
                    league.matches.iter_in_range(week_start, week_end),
                );
                let candidate =
                    AwardAggregator::aggregate(league.matches.iter_in_range(week_start, week_end));
                (league.id, weekly, candidate)
            })
            .collect();

        let mut weekly: HashMap<u32, HashMap<u32, WeeklyAggregate>> =
            HashMap::with_capacity(entries.len());
        let mut candidate: HashMap<u32, HashMap<u32, CandidateAggregate>> =
            HashMap::with_capacity(entries.len());
        for (lid, w, c) in entries {
            weekly.insert(lid, w);
            candidate.insert(lid, c);
        }
        MondayAwardCache { weekly, candidate }
    }

    fn weekly_for(&self, league_id: u32) -> Option<&HashMap<u32, WeeklyAggregate>> {
        self.weekly.get(&league_id)
    }

    fn candidate_for(&self, league_id: u32) -> Option<&HashMap<u32, CandidateAggregate>> {
        self.candidate.get(&league_id)
    }
}

/// Monday-only orchestration that walks every non-friendly league, picks
/// its Player of the Week from last calendar week's matches, and applies
/// the side effects (player happiness event + league archive). Two-pass
/// design avoids overlapping `&` and `&mut` borrows of `SimulatorData`.
struct WeeklyAwardsTick;

impl WeeklyAwardsTick {
    fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let week_end = data.date.date();
        let pending = Self::collect_pending(data, week_end, cache);
        Self::apply_pending(data, pending);
    }

    fn collect_pending(
        data: &SimulatorData,
        week_end: chrono::NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingWeeklyAward> {
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.player_of_week.has_award_for_week(week_end) {
                    return None;
                }

                let scores = cache.weekly_for(league.id)?;
                let (winner_id, agg) = PlayerOfTheWeekSelector::pick_winner(scores)?;

                let player = data.player(winner_id)?;
                let player_name = format!(
                    "{} {}",
                    player.full_name.display_first_name(),
                    player.full_name.display_last_name()
                );
                let player_slug = player.slug();
                let (club_id, club_name, club_slug) = Self::resolve_club_card(data, winner_id);
                let average_rating = if agg.matches_played > 0 {
                    agg.rating_sum / agg.matches_played as f32
                } else {
                    0.0
                };

                Some(PendingWeeklyAward {
                    league_id: league.id,
                    winner_id,
                    award: PlayerOfTheWeekAward {
                        week_end_date: week_end,
                        player_id: winner_id,
                        player_name,
                        player_slug,
                        club_id,
                        club_name,
                        club_slug,
                        score: agg.score,
                        goals: agg.goals,
                        assists: agg.assists,
                        matches_played: agg.matches_played,
                        average_rating,
                    },
                })
            })
            .collect()
    }

    fn apply_pending(data: &mut SimulatorData, pending: Vec<PendingWeeklyAward>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            let avg_rating = entry.award.average_rating;
            let matches_played = entry.award.matches_played;
            if let Some(player) = data.player_mut(entry.winner_id) {
                player.on_player_of_the_week();
                let mut input = AwardReputationInput::new()
                    .with_avg_rating(avg_rating)
                    .with_matches_played(matches_played as u16);
                if let Some(rep) = league_rep {
                    input = input.with_league_reputation(rep);
                }
                player.apply_award_reputation_impact(
                    AwardReputationKind::PlayerOfTheWeek,
                    input,
                    now,
                );
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.player_of_week.record(entry.award);
            }
        }
    }

    /// Resolve the active-club card (id, display name, slug) for a
    /// winning player. Falls back to empty values if the player isn't on
    /// a roster (free agent at award time — extremely unlikely
    /// Monday-morning, but guarded against).
    fn resolve_club_card(data: &SimulatorData, player_id: u32) -> (u32, String, String) {
        let location = data
            .indexes
            .as_ref()
            .and_then(|i| i.get_player_location(player_id));
        let Some((_, _, club_id, _)) = location else {
            return (0, String::new(), String::new());
        };
        let Some(club) = data.club(club_id) else {
            return (club_id, String::new(), String::new());
        };
        let main_team = club.teams.main();
        let club_name = main_team
            .map(|t| t.name.clone())
            .unwrap_or_else(|| club.name.clone());
        let club_slug = main_team
            .map(|t| t.slug.clone())
            .unwrap_or_else(String::new);
        (club_id, club_name, club_slug)
    }
}

struct PendingWeeklyAward {
    league_id: u32,
    winner_id: u32,
    award: PlayerOfTheWeekAward,
}

/// Monday-only Team of the Week selection. Builds an XI per league with
/// the canonical 1-4-4-2 quotas and emits `TeamOfTheWeekSelection` to each
/// selected player.
struct TeamOfTheWeekTick;

struct PendingTeamOfWeek {
    league_id: u32,
    award: TeamOfTheWeekAward,
}

impl TeamOfTheWeekTick {
    fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let week_end = data.date.date();
        let pending = Self::collect(data, week_end, cache);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        week_end: chrono::NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingTeamOfWeek> {
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_team_of_week_for(week_end) {
                    return None;
                }
                let scores = cache.candidate_for(league.id)?;
                let team = TeamOfTheWeekSelector::pick(scores);
                if team.is_empty() {
                    return None;
                }

                let mut slots: Vec<TeamOfTheWeekSlot> = Vec::with_capacity(team.len());
                for (pid, pos, score, agg) in team {
                    let Some(player) = data.player(pid) else {
                        continue;
                    };
                    let player_name = format!(
                        "{} {}",
                        player.full_name.display_first_name(),
                        player.full_name.display_last_name()
                    );
                    let player_slug = player.slug();
                    let (club_id, club_name, club_slug) =
                        WeeklyAwardsTick::resolve_club_card(data, pid);
                    slots.push(TeamOfTheWeekSlot {
                        player_id: pid,
                        player_name,
                        player_slug,
                        club_id,
                        club_name,
                        club_slug,
                        position_group: pos,
                        score,
                        matches_played: agg.matches_played,
                        goals: agg.goals,
                        assists: agg.assists,
                        average_rating: agg.average_rating(),
                    });
                }
                Some(PendingTeamOfWeek {
                    league_id: league.id,
                    award: TeamOfTheWeekAward {
                        week_end_date: week_end,
                        slots,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingTeamOfWeek>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            for slot in &entry.award.slots {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::TeamOfTheWeekSelection,
                        6,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::TeamOfTheWeekSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.awards.record_team_of_week(entry.award);
            }
        }
    }
}

/// Monday-only Young Player of the Week (age ≤ `YOUNG_WEEKLY_MAX_AGE`).
/// Mirrors `WeeklyAwardsTick` but filters the candidate set to under-20s
/// before scoring. Stored in `LeagueAwards::young_player_of_week`, not on
/// `League::player_of_week`, so the senior history stays untouched.
struct YoungWeeklyAwardsTick;

struct PendingYoungWeeklyAward {
    league_id: u32,
    winner_id: u32,
    award: PlayerOfTheWeekAward,
}

impl YoungWeeklyAwardsTick {
    fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let today = data.date.date();
        let pending = Self::collect(data, today, cache);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        today: chrono::NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingYoungWeeklyAward> {
        let week_end = today;
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_young_player_of_week_for(week_end) {
                    return None;
                }

                let scores = cache.weekly_for(league.id)?;
                // Score-then-filter: identical scoring to the senior
                // award, then the eligibility gate runs over the
                // candidate aggregate to keep the tiebreak deterministic.
                let mut young: HashMap<u32, WeeklyAggregate> = HashMap::new();
                for (id, agg) in scores {
                    if data
                        .player(*id)
                        .map(|p| DateUtils::age(p.birth_date, today) <= YOUNG_WEEKLY_MAX_AGE)
                        .unwrap_or(false)
                    {
                        young.insert(*id, *agg);
                    }
                }
                let (winner_id, agg) = PlayerOfTheWeekSelector::pick_winner(&young)?;

                let player = data.player(winner_id)?;
                let player_name = format!(
                    "{} {}",
                    player.full_name.display_first_name(),
                    player.full_name.display_last_name()
                );
                let player_slug = player.slug();
                let (club_id, club_name, club_slug) =
                    WeeklyAwardsTick::resolve_club_card(data, winner_id);
                let average_rating = if agg.matches_played > 0 {
                    agg.rating_sum / agg.matches_played as f32
                } else {
                    0.0
                };

                Some(PendingYoungWeeklyAward {
                    league_id: league.id,
                    winner_id,
                    award: PlayerOfTheWeekAward {
                        week_end_date: week_end,
                        player_id: winner_id,
                        player_name,
                        player_slug,
                        club_id,
                        club_name,
                        club_slug,
                        score: agg.score,
                        goals: agg.goals,
                        assists: agg.assists,
                        matches_played: agg.matches_played,
                        average_rating,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingYoungWeeklyAward>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            let avg_rating = entry.award.average_rating;
            let matches_played = entry.award.matches_played;
            if let Some(player) = data.player_mut(entry.winner_id) {
                player.on_young_player_of_the_week();
                let mut input = AwardReputationInput::new()
                    .with_avg_rating(avg_rating)
                    .with_matches_played(matches_played as u16);
                if let Some(rep) = league_rep {
                    input = input.with_league_reputation(rep);
                }
                player.apply_award_reputation_impact(
                    AwardReputationKind::YoungPlayerOfTheWeek,
                    input,
                    now,
                );
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.awards.record_young_player_of_week(entry.award);
            }
        }
    }
}

/// Monday-only Young Team of the Week. Reuses `TeamOfTheWeekSelector`
/// over the same week window but with the candidate set restricted to
/// players aged ≤ `YOUNG_WEEKLY_MAX_AGE` on the award date.
struct YoungTeamOfTheWeekTick;

struct PendingYoungTeamOfWeek {
    league_id: u32,
    award: TeamOfTheWeekAward,
}

impl YoungTeamOfTheWeekTick {
    fn run(data: &mut SimulatorData, cache: &MondayAwardCache) {
        let today = data.date.date();
        let pending = Self::collect(data, today, cache);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        today: chrono::NaiveDate,
        cache: &MondayAwardCache,
    ) -> Vec<PendingYoungTeamOfWeek> {
        let week_end = today;
        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_young_team_of_week_for(week_end) {
                    return None;
                }
                let scores = cache.candidate_for(league.id)?;
                let young: HashMap<u32, CandidateAggregate> = scores
                    .iter()
                    .filter(|(id, _)| {
                        data.player(**id)
                            .map(|p| DateUtils::age(p.birth_date, today) <= YOUNG_WEEKLY_MAX_AGE)
                            .unwrap_or(false)
                    })
                    .map(|(id, agg)| (*id, *agg))
                    .collect();
                let team = TeamOfTheWeekSelector::pick(&young);
                if team.is_empty() {
                    return None;
                }

                let mut slots: Vec<TeamOfTheWeekSlot> = Vec::with_capacity(team.len());
                for (pid, pos, score, agg) in team {
                    let Some(player) = data.player(pid) else {
                        continue;
                    };
                    let player_name = format!(
                        "{} {}",
                        player.full_name.display_first_name(),
                        player.full_name.display_last_name()
                    );
                    let player_slug = player.slug();
                    let (club_id, club_name, club_slug) =
                        WeeklyAwardsTick::resolve_club_card(data, pid);
                    slots.push(TeamOfTheWeekSlot {
                        player_id: pid,
                        player_name,
                        player_slug,
                        club_id,
                        club_name,
                        club_slug,
                        position_group: pos,
                        score,
                        matches_played: agg.matches_played,
                        goals: agg.goals,
                        assists: agg.assists,
                        average_rating: agg.average_rating(),
                    });
                }
                Some(PendingYoungTeamOfWeek {
                    league_id: league.id,
                    award: TeamOfTheWeekAward {
                        week_end_date: week_end,
                        slots,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingYoungTeamOfWeek>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            for slot in &entry.award.slots {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.happiness.add_event_default_with_cooldown(
                        HappinessEventType::YoungTeamOfTheWeekSelection,
                        6,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::YoungTeamOfTheWeekSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                league.awards.record_young_team_of_week(entry.award);
            }
        }
    }
}

/// Monthly awards — POM and Young POM per league plus a frozen
/// per-league `MonthlyAwardsSnapshot` (Team of Month, Young Team of
/// Month, top scorers / assists / ratings). Runs on the 1st of each
/// calendar month, awarding the *previous* calendar month.
///
/// Empty months (no non-friendly matches with stats) are skipped
/// entirely — no PoM, no snapshot, no `last_monthly_award` bump — so
/// the web layer's "latest monthly" view always shows the most recent
/// month that actually had fixtures (winter break / split-season /
/// summer-calendar leagues all behave correctly without any
/// per-league special-casing).
struct MonthlyAwardsTick;

struct PendingMonthlyAward {
    league_id: u32,
    pom: Option<MonthlyPlayerAward>,
    young: Option<MonthlyPlayerAward>,
    snapshot: MonthlyAwardsSnapshot,
}

const MONTHLY_TOP_N: usize = 5;
const MONTHLY_RATING_MIN_APPS: u8 = 2;

impl MonthlyAwardsTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if today.day() != 1 {
            return;
        }
        let (start, end) = match Self::previous_month_window(today) {
            Some(w) => w,
            None => return,
        };

        let pending = Self::collect(data, today, start, end);
        Self::apply(data, pending);
    }

    /// First-of-month → start = first of previous month, end = first of
    /// this month (exclusive in `iter_in_range`).
    fn previous_month_window(
        today: chrono::NaiveDate,
    ) -> Option<(chrono::NaiveDate, chrono::NaiveDate)> {
        let first_this_month = chrono::NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?;
        let prev_month = if today.month() == 1 {
            chrono::NaiveDate::from_ymd_opt(today.year() - 1, 12, 1)?
        } else {
            chrono::NaiveDate::from_ymd_opt(today.year(), today.month() - 1, 1)?
        };
        Some((prev_month, first_this_month))
    }

    fn collect(
        data: &SimulatorData,
        today: chrono::NaiveDate,
        start: chrono::NaiveDate,
        end: chrono::NaiveDate,
    ) -> Vec<PendingMonthlyAward> {
        let month_end = end - Duration::days(1);

        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_monthly_award_for(month_end) {
                    return None;
                }

                // Count non-friendly matches that produced stats. If
                // the previous calendar month had none (winter break,
                // off-season gap, league not started yet), skip the
                // league entirely — no snapshot, no last_monthly_award
                // bump — so the web view keeps showing the previously
                // archived month.
                let matches_count = league
                    .matches
                    .iter_in_range(start, end)
                    .filter(|m| !m.friendly && m.details.is_some())
                    .count() as u32;
                if matches_count == 0 {
                    return None;
                }

                let scores = AwardAggregator::aggregate(league.matches.iter_in_range(start, end));

                let pom = Self::pick_pom(data, &scores, league.reputation, month_end);
                let young =
                    Self::pick_young_pom(data, &scores, league.reputation, today, month_end);

                let team_of_month = Self::build_team(data, &scores, |_| true);
                let young_team_of_month = Self::build_team(data, &scores, |id| {
                    data.player(id)
                        .map(|p| DateUtils::age(p.birth_date, today) <= 21)
                        .unwrap_or(false)
                });

                let top_scorers = Self::top_scorers(data, &scores);
                let top_assists = Self::top_assists(data, &scores);
                let best_ratings = Self::best_ratings(data, &scores);

                let snapshot = MonthlyAwardsSnapshot {
                    month_start_date: start,
                    month_end_date: month_end,
                    matches_count,
                    player_of_month: pom.clone(),
                    young_player_of_month: young.clone(),
                    team_of_month,
                    young_team_of_month,
                    top_scorers,
                    top_assists,
                    best_ratings,
                };

                Some(PendingMonthlyAward {
                    league_id: league.id,
                    pom,
                    young,
                    snapshot,
                })
            })
            .collect()
    }

    fn pick_pom(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
        league_reputation: u16,
        month_end: chrono::NaiveDate,
    ) -> Option<MonthlyPlayerAward> {
        let (id, agg, score) =
            MonthlyAwardSelector::pick_best(scores, league_reputation, 3, |_| true)?;
        Self::monthly_award(data, id, agg, score, month_end)
    }

    fn pick_young_pom(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
        league_reputation: u16,
        today: chrono::NaiveDate,
        month_end: chrono::NaiveDate,
    ) -> Option<MonthlyPlayerAward> {
        let (id, agg, score) =
            MonthlyAwardSelector::pick_best(scores, league_reputation, 2, |id| {
                data.player(id)
                    .map(|p| DateUtils::age(p.birth_date, today) <= 21)
                    .unwrap_or(false)
            })?;
        Self::monthly_award(data, id, agg, score, month_end)
    }

    fn monthly_award(
        data: &SimulatorData,
        id: u32,
        agg: CandidateAggregate,
        score: f32,
        month_end: chrono::NaiveDate,
    ) -> Option<MonthlyPlayerAward> {
        let player = data.player(id)?;
        let (club_id, club_name, club_slug) = WeeklyAwardsTick::resolve_club_card(data, id);
        Some(MonthlyPlayerAward {
            month_end_date: month_end,
            player_id: id,
            player_name: format!(
                "{} {}",
                player.full_name.display_first_name(),
                player.full_name.display_last_name()
            ),
            player_slug: player.slug(),
            club_id,
            club_name,
            club_slug,
            matches_played: agg.matches_played,
            goals: agg.goals,
            assists: agg.assists,
            average_rating: agg.average_rating(),
            score,
        })
    }

    /// Pick a Team of Month (1-4-4-2 quotas, min 2 apps) from the
    /// passed aggregate, restricted to ids passing `eligibility`. Used
    /// for both the open and Young (≤21) variants.
    fn build_team(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
        eligibility: impl Fn(u32) -> bool,
    ) -> Vec<TeamOfTheWeekSlot> {
        let filtered: HashMap<u32, CandidateAggregate> = scores
            .iter()
            .filter(|(id, _)| eligibility(**id))
            .map(|(id, a)| (*id, *a))
            .collect();
        if filtered.is_empty() {
            return Vec::new();
        }
        let team = TeamOfTheWeekSelector::pick_with_min_apps(&filtered, 2);
        let mut slots: Vec<TeamOfTheWeekSlot> = Vec::with_capacity(team.len());
        for (pid, pos, score, agg) in team {
            let Some(player) = data.player(pid) else {
                continue;
            };
            let player_name = format!(
                "{} {}",
                player.full_name.display_first_name(),
                player.full_name.display_last_name()
            );
            let player_slug = player.slug();
            let (club_id, club_name, club_slug) = WeeklyAwardsTick::resolve_club_card(data, pid);
            slots.push(TeamOfTheWeekSlot {
                player_id: pid,
                player_name,
                player_slug,
                club_id,
                club_name,
                club_slug,
                position_group: pos,
                score,
                matches_played: agg.matches_played,
                goals: agg.goals,
                assists: agg.assists,
                average_rating: agg.average_rating(),
            });
        }
        slots
    }

    fn top_scorers(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
    ) -> Vec<MonthlyStatLeader> {
        let mut all: Vec<(u32, CandidateAggregate)> = scores
            .iter()
            .filter(|(_, a)| a.goals > 0)
            .map(|(id, a)| (*id, *a))
            .collect();
        all.sort_by(|(la, aa), (lb, ab)| {
            ab.goals
                .cmp(&aa.goals)
                .then(ab.assists.cmp(&aa.assists))
                .then(
                    ab.average_rating()
                        .partial_cmp(&aa.average_rating())
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(la.cmp(lb))
        });
        all.into_iter()
            .take(MONTHLY_TOP_N)
            .filter_map(|(id, agg)| Self::stat_leader(data, id, agg))
            .collect()
    }

    fn top_assists(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
    ) -> Vec<MonthlyStatLeader> {
        let mut all: Vec<(u32, CandidateAggregate)> = scores
            .iter()
            .filter(|(_, a)| a.assists > 0)
            .map(|(id, a)| (*id, *a))
            .collect();
        all.sort_by(|(la, aa), (lb, ab)| {
            ab.assists
                .cmp(&aa.assists)
                .then(ab.goals.cmp(&aa.goals))
                .then(
                    ab.average_rating()
                        .partial_cmp(&aa.average_rating())
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(la.cmp(lb))
        });
        all.into_iter()
            .take(MONTHLY_TOP_N)
            .filter_map(|(id, agg)| Self::stat_leader(data, id, agg))
            .collect()
    }

    fn best_ratings(
        data: &SimulatorData,
        scores: &HashMap<u32, CandidateAggregate>,
    ) -> Vec<MonthlyStatLeader> {
        let mut all: Vec<(u32, CandidateAggregate)> = scores
            .iter()
            .filter(|(_, a)| a.matches_played >= MONTHLY_RATING_MIN_APPS)
            .map(|(id, a)| (*id, *a))
            .collect();
        all.sort_by(|(la, aa), (lb, ab)| {
            ab.average_rating()
                .partial_cmp(&aa.average_rating())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(ab.matches_played.cmp(&aa.matches_played))
                .then(ab.goals.cmp(&aa.goals))
                .then(la.cmp(lb))
        });
        all.into_iter()
            .take(MONTHLY_TOP_N)
            .filter_map(|(id, agg)| Self::stat_leader(data, id, agg))
            .collect()
    }

    fn stat_leader(
        data: &SimulatorData,
        id: u32,
        agg: CandidateAggregate,
    ) -> Option<MonthlyStatLeader> {
        let player = data.player(id)?;
        let (club_id, club_name, club_slug) = WeeklyAwardsTick::resolve_club_card(data, id);
        Some(MonthlyStatLeader {
            player_id: id,
            player_name: format!(
                "{} {}",
                player.full_name.display_first_name(),
                player.full_name.display_last_name()
            ),
            player_slug: player.slug(),
            club_id,
            club_name,
            club_slug,
            position_group: agg
                .primary_position
                .unwrap_or(PlayerFieldPositionGroup::Midfielder),
            matches_played: agg.matches_played,
            goals: agg.goals,
            assists: agg.assists,
            average_rating: agg.average_rating(),
        })
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingMonthlyAward>) {
        let now = data.date.date();
        for entry in pending {
            let league_rep = data.league(entry.league_id).map(|l| l.reputation);
            // Young POM fires before senior POM so the centralised
            // award-reputation pipeline dampens the senior emit when
            // the same player wins both — young base is larger, so it
            // takes the full impact.
            if let Some(award) = &entry.young {
                let ctx = Self::pom_context(
                    RecognitionEventKind::YoungPlayerOfTheMonth,
                    entry.league_id,
                    award,
                );
                let avg_rating = award.average_rating;
                let matches_played = award.matches_played;
                if let Some(player) = data.player_mut(award.player_id) {
                    player.on_recognition_award(HappinessEventType::YoungPlayerOfTheMonth, ctx, 28);
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::YoungPlayerOfTheMonth,
                        input,
                        now,
                    );
                }
            }
            if let Some(award) = &entry.pom {
                let ctx = Self::pom_context(
                    RecognitionEventKind::PlayerOfTheMonth,
                    entry.league_id,
                    award,
                );
                let avg_rating = award.average_rating;
                let matches_played = award.matches_played;
                if let Some(player) = data.player_mut(award.player_id) {
                    player.on_recognition_award(HappinessEventType::PlayerOfTheMonth, ctx, 28);
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::PlayerOfTheMonth,
                        input,
                        now,
                    );
                }
            }
            // Team-of-month XIs fire after the individual monthly
            // awards so the centralised stacking dampener can suppress
            // the team-XI reputation gain when the same player just
            // won POM / Young POM. Young XI runs first by the same
            // larger-base-takes-full-impact rule used for POW/TOTW.
            for slot in &entry.snapshot.young_team_of_month {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                let ctx = RecognitionEventContext::new(
                    RecognitionEventKind::YoungTeamOfTheMonthSelection,
                )
                .with_league(entry.league_id)
                .with_avg_rating(avg_rating)
                .with_matches_played(matches_played as u16)
                .with_season_goals(slot.goals as u16)
                .with_season_assists(slot.assists as u16);
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.on_recognition_award(
                        HappinessEventType::YoungTeamOfTheMonthSelection,
                        ctx,
                        28,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::YoungTeamOfTheMonthSelection,
                        input,
                        now,
                    );
                }
            }
            for slot in &entry.snapshot.team_of_month {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                let ctx =
                    RecognitionEventContext::new(RecognitionEventKind::TeamOfTheMonthSelection)
                        .with_league(entry.league_id)
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16)
                        .with_season_goals(slot.goals as u16)
                        .with_season_assists(slot.assists as u16);
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.on_recognition_award(
                        HappinessEventType::TeamOfTheMonthSelection,
                        ctx,
                        28,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::TeamOfTheMonthSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(entry.league_id) {
                if let Some(a) = entry.pom {
                    league.awards.record_player_of_month(a);
                }
                if let Some(a) = entry.young {
                    league.awards.record_young_player_of_month(a);
                }
                league.awards.record_monthly_snapshot(entry.snapshot);
            }
        }
    }

    fn pom_context(
        kind: RecognitionEventKind,
        league_id: u32,
        award: &MonthlyPlayerAward,
    ) -> RecognitionEventContext {
        RecognitionEventContext::new(kind)
            .with_league(league_id)
            .with_season_goals(award.goals as u16)
            .with_season_assists(award.assists as u16)
            .with_avg_rating(award.average_rating)
            .with_matches_played(award.matches_played as u16)
    }
}

/// Season awards — drains each league's pending snapshot (built inside
/// `process_season_end` before stats archive) and fires player events.
struct SeasonAwardsTick;

impl SeasonAwardsTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        let pending: Vec<(u32, SeasonAwardsSnapshot)> = data
            .continents
            .par_iter_mut()
            .flat_map(|c| c.countries.par_iter_mut())
            .flat_map(|c| c.leagues.leagues.par_iter_mut())
            .filter_map(|l| l.awards.pending_season_awards.take().map(|s| (l.id, s)))
            .collect();

        for (league_id, snapshot) in pending {
            let league_rep = data.league(league_id).map(|l| l.reputation);

            // Young POS first so the senior-POS emit is dampened when
            // the same player wins both in a single season.
            if let Some(id) = snapshot.young_player_of_season {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::YoungPlayerOfTheSeason,
                    RecognitionEventKind::YoungPlayerOfTheSeason,
                    AwardReputationKind::YoungPlayerOfTheSeason,
                );
            }
            if let Some(id) = snapshot.player_of_season {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::PlayerOfTheSeason,
                    RecognitionEventKind::PlayerOfTheSeason,
                    AwardReputationKind::PlayerOfTheSeason,
                );
            }
            for pid in &snapshot.team_of_season {
                Self::apply_player_award(
                    data,
                    *pid,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::TeamOfTheSeasonSelection,
                    RecognitionEventKind::TeamOfTheSeasonSelection,
                    AwardReputationKind::TeamOfTheSeasonSelection,
                );
            }
            if let Some(id) = snapshot.top_scorer {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::LeagueTopScorer,
                    RecognitionEventKind::LeagueTopScorer,
                    AwardReputationKind::LeagueTopScorer,
                );
            }
            if let Some(id) = snapshot.top_assists {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::LeagueTopAssists,
                    RecognitionEventKind::LeagueTopAssists,
                    AwardReputationKind::LeagueTopAssists,
                );
            }
            if let Some(id) = snapshot.golden_glove {
                Self::apply_player_award(
                    data,
                    id,
                    league_id,
                    league_rep,
                    today,
                    HappinessEventType::LeagueGoldenGlove,
                    RecognitionEventKind::LeagueGoldenGlove,
                    AwardReputationKind::LeagueGoldenGlove,
                );
            }
            // Archive the snapshot once events have been applied.
            let mut snapshot = snapshot;
            snapshot.season_end_date = today;
            if let Some(league) = data.league_mut(league_id) {
                league.awards.record_season(snapshot);
            }
        }
    }

    /// Apply a season-end award: emit the recognition event with full
    /// season context (avg rating, matches, goals/assists) and route
    /// the centralised reputation impact through the same helper used
    /// by every other emit site, so the model is league-aware,
    /// quality-weighted, and headroom-bounded in one place.
    fn apply_player_award(
        data: &mut SimulatorData,
        player_id: u32,
        league_id: u32,
        league_rep: Option<u16>,
        now: chrono::NaiveDate,
        happiness_event: HappinessEventType,
        recognition_kind: RecognitionEventKind,
        reputation_kind: AwardReputationKind,
    ) {
        let Some(player) = data.player_mut(player_id) else {
            return;
        };
        let avg_rating = player.statistics.average_rating;
        let matches_played = player.statistics.played + player.statistics.played_subs;
        let goals = player.statistics.goals;
        let assists = player.statistics.assists;
        let mut ctx = RecognitionEventContext::new(recognition_kind).with_league(league_id);
        if matches_played > 0 {
            ctx = ctx
                .with_avg_rating(avg_rating)
                .with_matches_played(matches_played as u16)
                .with_season_goals(goals as u16)
                .with_season_assists(assists as u16);
        }
        player.on_recognition_award(happiness_event, ctx, 330);

        let mut input = AwardReputationInput::new();
        if let Some(rep) = league_rep {
            input = input.with_league_reputation(rep);
        }
        if matches_played > 0 {
            input = input
                .with_avg_rating(avg_rating)
                .with_matches_played(matches_played as u16);
        }
        player.apply_award_reputation_impact(reputation_kind, input, now);
    }
}

/// Calendar-year XI per league. Runs once on December 31. Aggregates
/// every non-friendly league match played between Jan 1 (inclusive)
/// and Jan 1 of the next year (exclusive) and picks an XI with the
/// canonical Team of the Week quotas, gated by a per-player minimum
/// appearances threshold so a one-match wonder cannot win.
///
/// Distinct from `team_of_season` (which aligns to the league's
/// season end) — this archive is calendar-year aligned and lives at
/// `LeagueAwards::team_of_year`.
struct TeamOfTheYearTick;

struct PendingTeamOfYear {
    league_id: u32,
    award: TeamOfTheYearAward,
}

impl TeamOfTheYearTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if !DateUtils::is_year_end(today) {
            return;
        }
        let year = today.year();
        let pending = Self::collect(data, today, year);
        Self::apply(data, pending);
    }

    fn collect(
        data: &SimulatorData,
        year_end_date: chrono::NaiveDate,
        year: i32,
    ) -> Vec<PendingTeamOfYear> {
        let year_start = chrono::NaiveDate::from_ymd_opt(year, 1, 1).unwrap_or(year_end_date);
        let next_year_start =
            chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap_or(year_end_date);

        data.continents
            .par_iter()
            .flat_map(|c| c.countries.par_iter())
            .flat_map(|country| country.leagues.leagues.par_iter())
            .filter_map(|league| {
                if league.friendly {
                    return None;
                }
                if league.awards.has_team_of_year_for(year) {
                    return None;
                }

                let scores = AwardAggregator::aggregate(
                    league.matches.iter_in_range(year_start, next_year_start),
                );
                if scores.is_empty() {
                    return None;
                }

                // Year-level appearance gate: max(10, ~25% of typical
                // matches per team). Calendar year ≈ one full league
                // campaign for most leagues, but split-season /
                // summer leagues may straddle two campaigns inside
                // one calendar year — the percentage scales with
                // whatever fixture density actually fell in this
                // window so a thin schedule isn't impossibly gated.
                let team_count = league.table.rows.len() as u32;
                let typical_matches_per_team = team_count.saturating_sub(1) * 2;
                let pct_floor = ((typical_matches_per_team as f32) * 0.25).round() as u8;
                let min_apps = pct_floor.max(10);

                let team = TeamOfTheWeekSelector::pick_with_min_apps(&scores, min_apps);
                if team.is_empty() {
                    return None;
                }

                let mut slots: Vec<TeamOfTheWeekSlot> = Vec::with_capacity(team.len());
                for (pid, pos, score, agg) in team {
                    let Some(player) = data.player(pid) else {
                        continue;
                    };
                    let player_name = format!(
                        "{} {}",
                        player.full_name.display_first_name(),
                        player.full_name.display_last_name()
                    );
                    let player_slug = player.slug();
                    let (club_id, club_name, club_slug) =
                        WeeklyAwardsTick::resolve_club_card(data, pid);
                    slots.push(TeamOfTheWeekSlot {
                        player_id: pid,
                        player_name,
                        player_slug,
                        club_id,
                        club_name,
                        club_slug,
                        position_group: pos,
                        score,
                        matches_played: agg.matches_played,
                        goals: agg.goals,
                        assists: agg.assists,
                        average_rating: agg.average_rating(),
                    });
                }
                Some(PendingTeamOfYear {
                    league_id: league.id,
                    award: TeamOfTheYearAward {
                        year,
                        year_end_date,
                        slots,
                    },
                })
            })
            .collect()
    }

    fn apply(data: &mut SimulatorData, pending: Vec<PendingTeamOfYear>) {
        let now = data.date.date();
        for entry in pending {
            let league_id = entry.league_id;
            let league_rep = data.league(league_id).map(|l| l.reputation);
            for slot in &entry.award.slots {
                let avg_rating = slot.average_rating;
                let matches_played = slot.matches_played;
                if let Some(player) = data.player_mut(slot.player_id) {
                    player.on_recognition_award(
                        HappinessEventType::TeamOfTheYearSelection,
                        RecognitionEventContext::new(RecognitionEventKind::TeamOfTheYearSelection)
                            .with_league(league_id)
                            .with_avg_rating(avg_rating)
                            .with_matches_played(matches_played as u16),
                        330,
                    );
                    let mut input = AwardReputationInput::new()
                        .with_avg_rating(avg_rating)
                        .with_matches_played(matches_played as u16);
                    if let Some(rep) = league_rep {
                        input = input.with_league_reputation(rep);
                    }
                    player.apply_award_reputation_impact(
                        AwardReputationKind::TeamOfTheYearSelection,
                        input,
                        now,
                    );
                }
            }
            if let Some(league) = data.league_mut(league_id) {
                league.awards.record_team_of_year(entry.award);
            }
        }
    }
}

/// World player-of-year. Runs once on year-end. Pools each continent's
/// ranking, picks the global top 3 (nominees) and the global #1
/// (winner). Reuses `ContinentResult::rank_continent` so the scoring
/// formula has a single source of truth.
struct WorldPlayerOfYearTick;

impl WorldPlayerOfYearTick {
    fn run(data: &mut SimulatorData) {
        let today = data.date.date();
        if !DateUtils::is_year_end(today) {
            return;
        }

        let mut combined: Vec<(u32, f32)> = data
            .continents
            .par_iter()
            .flat_map_iter(|c| ContinentResult::rank_continent(c, today))
            .collect();
        combined.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        let top_three: Vec<u32> = combined.iter().take(3).map(|(id, _)| *id).collect();
        let winner = combined.first().map(|(id, _)| *id);

        let runner_up_id = combined.get(1).map(|(id, _)| *id);
        let winner_score = combined.first().map(|(_, score)| *score);
        let winner_margin = combined
            .first()
            .zip(combined.get(1))
            .map(|((_, w), (_, r))| (*w - *r).max(0.0));

        for pid in &top_three {
            if let Some(player) = data.player_mut(*pid) {
                let mut ctx =
                    RecognitionEventContext::new(RecognitionEventKind::WorldPlayerOfYearNomination);
                if let Some(rup) = runner_up_id {
                    ctx = ctx.with_runner_up(rup);
                }
                player.on_recognition_award(
                    HappinessEventType::WorldPlayerOfYearNomination,
                    ctx,
                    330,
                );
            }
        }
        if let Some(id) = winner {
            if let Some(player) = data.player_mut(id) {
                let mut ctx = RecognitionEventContext::new(RecognitionEventKind::WorldPlayerOfYear);
                if let Some(rup) = runner_up_id {
                    ctx = ctx.with_runner_up(rup);
                }
                if let Some(margin) = winner_margin {
                    ctx = ctx.with_margin(margin);
                }
                if let Some(score) = winner_score {
                    ctx = ctx.with_avg_rating(score);
                }
                player.on_recognition_award(HappinessEventType::WorldPlayerOfYear, ctx, 330);
                player.apply_award_reputation_impact(
                    AwardReputationKind::WorldPlayerOfYear,
                    AwardReputationInput::new(),
                    today,
                );
            }
        }
    }
}

// ============================================================
// Internal: league-seeding helpers
// ============================================================

/// Collect every team id that participates in `league_id` across the
/// given clubs. Extracted so `init_league_tables`'s outer loop reads as
/// "for each league, install a table from the team ids" without an inline
/// `flat_map` chain.
fn team_ids_for_league(clubs: &[crate::Club], league_id: u32) -> Vec<u32> {
    clubs
        .iter()
        .flat_map(|c| c.teams.with_league(league_id))
        .collect()
}

// ============================================================
// Internal: history-seeding helpers
// ============================================================

/// Per-country `league_id -> (name, slug)` cache. Built once at the start
/// of a country's seeding sweep so the per-club main-team lookup is O(1).
fn build_league_lookup(country: &crate::Country) -> HashMap<u32, (String, String)> {
    country
        .leagues
        .leagues
        .iter()
        .map(|l| (l.id, (l.name.clone(), l.slug.clone())))
        .collect()
}

/// True if any team in the club has at least one player needing a current-
/// season seed entry. Cheap traversal — exits as soon as one is found.
fn club_has_players_needing_seed(club: &crate::Club) -> bool {
    club.teams.iter().any(|t| team_has_players_needing_seed(t))
}

fn team_has_players_needing_seed(team: &crate::club::Team) -> bool {
    team.players
        .iter()
        .any(|p| p.statistics_history.needs_current_season_seed())
}

/// Snapshot of the club's main-team identity for stats-seeding purposes.
/// Resolved once per club so youth teams (U18-U23) and Reserve inherit the
/// main brand consistently across all their players. Senior reserves
/// (B, Second) keep their own identity because they compete in real
/// lower divisions and players' histories should show that.
struct ClubSeedingContext {
    main_name: Option<String>,
    main_slug: Option<String>,
    main_reputation: u16,
    main_league_name: String,
    main_league_slug: String,
    league_lookup: HashMap<u32, (String, String)>,
}

impl ClubSeedingContext {
    fn resolve(club: &crate::Club, league_lookup: &HashMap<u32, (String, String)>) -> Self {
        let main_team = club.teams.main();
        let main_name = main_team.map(|t| t.name.clone());
        let main_slug = main_team.map(|t| t.slug.clone());
        let main_reputation = main_team.map(|t| t.reputation.world).unwrap_or(0);
        let (main_league_name, main_league_slug) = main_team
            .and_then(|t| t.league_id)
            .and_then(|lid| league_lookup.get(&lid))
            .map(|(n, s)| (n.clone(), s.clone()))
            .unwrap_or_default();
        ClubSeedingContext {
            main_name,
            main_slug,
            main_reputation,
            main_league_name,
            main_league_slug,
            league_lookup: league_lookup.clone(),
        }
    }

    /// Build the `TeamInfo` that the seeder writes onto the player's
    /// history. Main, B and Second teams keep their own identity (each
    /// competes in a real league); youth and Reserve squads inherit the
    /// main brand so synthetic sub-league stats aggregate under the club.
    fn team_info_for(&self, team: &crate::club::Team) -> TeamInfo {
        let keeps_own_identity = team.team_type.is_own_team();
        if keeps_own_identity {
            let (league_name, league_slug) = team
                .league_id
                .and_then(|lid| self.league_lookup.get(&lid))
                .cloned()
                .unwrap_or_else(|| (self.main_league_name.clone(), self.main_league_slug.clone()));
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name,
                league_slug,
            }
        } else if self.main_name.is_some() {
            TeamInfo {
                name: self.main_name.clone().unwrap_or_default(),
                slug: self.main_slug.clone().unwrap_or_default(),
                reputation: self.main_reputation,
                league_name: self.main_league_name.clone(),
                league_slug: self.main_league_slug.clone(),
            }
        } else {
            // Club has no main team at all — fall back to the team's own info.
            TeamInfo {
                name: team.name.clone(),
                slug: team.slug.clone(),
                reputation: team.reputation.world,
                league_name: self.main_league_name.clone(),
                league_slug: self.main_league_slug.clone(),
            }
        }
    }
}
