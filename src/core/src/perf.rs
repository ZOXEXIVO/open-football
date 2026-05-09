use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Phases of `FootballSimulator::simulate_with` that the dashboard
/// surfaces. The variant name is the source of truth for which "current"
/// atomic the timer feeds and which "last" atomic the snapshot reads.
#[derive(Clone, Copy, Debug)]
pub enum PerfPhase {
    WorldCallups,
    WorldNationalMatches,
    ParallelContinents,
    AiBatch,
    ResultProcessing,
    ManagerMarket,
    GlobalCompetitions,
    Cleanup,
    Awards,
}

/// Process-wide performance counters. One instance lives behind
/// `PerfCounters::instance()`. Every per-tick scratch field is reset by
/// `begin_tick()`; finalised values land in the `last_*` set inside
/// `end_tick()` so concurrent readers always observe a consistent
/// snapshot from the previous completed tick.
pub struct PerfCounters {
    cur_tick_start_ns: AtomicU64,

    cur_callups_ns: AtomicU64,
    cur_world_matches_ns: AtomicU64,
    cur_continents_ns: AtomicU64,
    cur_ai_batch_ns: AtomicU64,
    cur_ai_active: AtomicBool,
    cur_result_proc_ns: AtomicU64,
    cur_manager_market_ns: AtomicU64,
    cur_global_comp_ns: AtomicU64,
    cur_cleanup_ns: AtomicU64,
    cur_awards_ns: AtomicU64,

    cur_match_sim_ns: AtomicU64,
    cur_match_count: AtomicU64,
    cur_match_tick_ns: AtomicU64,
    cur_match_tick_count: AtomicU64,
    cur_match_result_proc_ns: AtomicU64,

    cur_dirty_index_rebuild: AtomicBool,

    last_total_ns: AtomicU64,
    last_was_match_day: AtomicBool,
    last_match_day_ns: AtomicU64,
    last_non_match_day_ns: AtomicU64,

    last_callups_ns: AtomicU64,
    last_world_matches_ns: AtomicU64,
    last_continents_ns: AtomicU64,
    last_ai_batch_ns: AtomicU64,
    last_ai_active: AtomicBool,
    last_result_proc_ns: AtomicU64,
    last_manager_market_ns: AtomicU64,
    last_global_comp_ns: AtomicU64,
    last_cleanup_ns: AtomicU64,
    last_awards_ns: AtomicU64,
    last_match_storage_ns: AtomicU64,

    last_match_sim_avg_ns: AtomicU64,
    last_match_tick_avg_ns: AtomicU64,
    last_match_count: AtomicU64,
    last_match_result_proc_ns: AtomicU64,

    simulated_days_total: AtomicU64,
    matches_simulated_total: AtomicU64,

    last_matches_simulated: AtomicU64,
    last_countries_processed: AtomicU64,
    last_leagues_processed: AtomicU64,
    last_clubs_processed: AtomicU64,
    last_players_touched: AtomicU64,
    last_match_results_written: AtomicU64,
    last_panicked_continents: AtomicU64,
    last_dirty_index_rebuild: AtomicBool,
    last_recording_mode: AtomicBool,

    process_start: OnceLock<Instant>,
    has_run: AtomicBool,
}

impl PerfCounters {
    pub fn instance() -> &'static PerfCounters {
        static INSTANCE: OnceLock<PerfCounters> = OnceLock::new();
        INSTANCE.get_or_init(PerfCounters::new)
    }

    fn new() -> Self {
        PerfCounters {
            cur_tick_start_ns: AtomicU64::new(0),
            cur_callups_ns: AtomicU64::new(0),
            cur_world_matches_ns: AtomicU64::new(0),
            cur_continents_ns: AtomicU64::new(0),
            cur_ai_batch_ns: AtomicU64::new(0),
            cur_ai_active: AtomicBool::new(false),
            cur_result_proc_ns: AtomicU64::new(0),
            cur_manager_market_ns: AtomicU64::new(0),
            cur_global_comp_ns: AtomicU64::new(0),
            cur_cleanup_ns: AtomicU64::new(0),
            cur_awards_ns: AtomicU64::new(0),
            cur_match_sim_ns: AtomicU64::new(0),
            cur_match_count: AtomicU64::new(0),
            cur_match_tick_ns: AtomicU64::new(0),
            cur_match_tick_count: AtomicU64::new(0),
            cur_match_result_proc_ns: AtomicU64::new(0),
            cur_dirty_index_rebuild: AtomicBool::new(false),

            last_total_ns: AtomicU64::new(0),
            last_was_match_day: AtomicBool::new(false),
            last_match_day_ns: AtomicU64::new(0),
            last_non_match_day_ns: AtomicU64::new(0),
            last_callups_ns: AtomicU64::new(0),
            last_world_matches_ns: AtomicU64::new(0),
            last_continents_ns: AtomicU64::new(0),
            last_ai_batch_ns: AtomicU64::new(0),
            last_ai_active: AtomicBool::new(false),
            last_result_proc_ns: AtomicU64::new(0),
            last_manager_market_ns: AtomicU64::new(0),
            last_global_comp_ns: AtomicU64::new(0),
            last_cleanup_ns: AtomicU64::new(0),
            last_awards_ns: AtomicU64::new(0),
            last_match_storage_ns: AtomicU64::new(0),
            last_match_sim_avg_ns: AtomicU64::new(0),
            last_match_tick_avg_ns: AtomicU64::new(0),
            last_match_count: AtomicU64::new(0),
            last_match_result_proc_ns: AtomicU64::new(0),
            simulated_days_total: AtomicU64::new(0),
            matches_simulated_total: AtomicU64::new(0),
            last_matches_simulated: AtomicU64::new(0),
            last_countries_processed: AtomicU64::new(0),
            last_leagues_processed: AtomicU64::new(0),
            last_clubs_processed: AtomicU64::new(0),
            last_players_touched: AtomicU64::new(0),
            last_match_results_written: AtomicU64::new(0),
            last_panicked_continents: AtomicU64::new(0),
            last_dirty_index_rebuild: AtomicBool::new(false),
            last_recording_mode: AtomicBool::new(false),
            process_start: OnceLock::new(),
            has_run: AtomicBool::new(false),
        }
    }

    fn now_ns(&self) -> u64 {
        let start = self.process_start.get_or_init(Instant::now);
        start.elapsed().as_nanos() as u64
    }

    pub fn begin_tick(&self) {
        self.cur_tick_start_ns
            .store(self.now_ns(), Ordering::Relaxed);
        self.cur_callups_ns.store(0, Ordering::Relaxed);
        self.cur_world_matches_ns.store(0, Ordering::Relaxed);
        self.cur_continents_ns.store(0, Ordering::Relaxed);
        self.cur_ai_batch_ns.store(0, Ordering::Relaxed);
        self.cur_ai_active.store(false, Ordering::Relaxed);
        self.cur_result_proc_ns.store(0, Ordering::Relaxed);
        self.cur_manager_market_ns.store(0, Ordering::Relaxed);
        self.cur_global_comp_ns.store(0, Ordering::Relaxed);
        self.cur_cleanup_ns.store(0, Ordering::Relaxed);
        self.cur_awards_ns.store(0, Ordering::Relaxed);
        self.cur_match_sim_ns.store(0, Ordering::Relaxed);
        self.cur_match_count.store(0, Ordering::Relaxed);
        self.cur_match_tick_ns.store(0, Ordering::Relaxed);
        self.cur_match_tick_count.store(0, Ordering::Relaxed);
        self.cur_match_result_proc_ns.store(0, Ordering::Relaxed);
        self.cur_dirty_index_rebuild.store(false, Ordering::Relaxed);
    }

    pub fn end_tick(&self, ctx: TickEndContext) {
        let total_ns =
            self.now_ns() - self.cur_tick_start_ns.load(Ordering::Relaxed);
        let was_match_day = ctx.match_results_written > 0
            || self.cur_match_count.load(Ordering::Relaxed) > 0;

        let match_count = self.cur_match_count.load(Ordering::Relaxed);
        let tick_count = self.cur_match_tick_count.load(Ordering::Relaxed);
        let match_sim_total = self.cur_match_sim_ns.load(Ordering::Relaxed);
        let tick_sim_total = self.cur_match_tick_ns.load(Ordering::Relaxed);

        let match_sim_avg = if match_count > 0 {
            match_sim_total / match_count
        } else {
            0
        };
        let tick_avg = if tick_count > 0 {
            tick_sim_total / tick_count
        } else {
            0
        };

        self.last_total_ns.store(total_ns, Ordering::Relaxed);
        self.last_was_match_day
            .store(was_match_day, Ordering::Relaxed);
        if was_match_day {
            self.last_match_day_ns.store(total_ns, Ordering::Relaxed);
        } else {
            self.last_non_match_day_ns
                .store(total_ns, Ordering::Relaxed);
        }

        self.last_callups_ns.store(
            self.cur_callups_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_world_matches_ns.store(
            self.cur_world_matches_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_continents_ns.store(
            self.cur_continents_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_ai_batch_ns.store(
            self.cur_ai_batch_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_ai_active.store(
            self.cur_ai_active.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_result_proc_ns.store(
            self.cur_result_proc_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_manager_market_ns.store(
            self.cur_manager_market_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_global_comp_ns.store(
            self.cur_global_comp_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_cleanup_ns.store(
            self.cur_cleanup_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_awards_ns.store(
            self.cur_awards_ns.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );

        self.last_match_count.store(match_count, Ordering::Relaxed);
        self.last_match_sim_avg_ns
            .store(match_sim_avg, Ordering::Relaxed);
        self.last_match_tick_avg_ns
            .store(tick_avg, Ordering::Relaxed);
        let result_proc_total = self.cur_match_result_proc_ns.load(Ordering::Relaxed);
        let result_proc_avg = if match_count > 0 {
            result_proc_total / match_count
        } else {
            0
        };
        self.last_match_result_proc_ns
            .store(result_proc_avg, Ordering::Relaxed);

        self.simulated_days_total.fetch_add(1, Ordering::Relaxed);
        self.matches_simulated_total
            .fetch_add(match_count, Ordering::Relaxed);

        self.last_matches_simulated
            .store(match_count, Ordering::Relaxed);
        self.last_countries_processed
            .store(ctx.countries, Ordering::Relaxed);
        self.last_leagues_processed
            .store(ctx.leagues, Ordering::Relaxed);
        self.last_clubs_processed.store(ctx.clubs, Ordering::Relaxed);
        self.last_players_touched
            .store(ctx.players, Ordering::Relaxed);
        self.last_match_results_written
            .store(ctx.match_results_written, Ordering::Relaxed);
        self.last_panicked_continents
            .store(ctx.panicked_continents as u64, Ordering::Relaxed);
        self.last_dirty_index_rebuild.store(
            self.cur_dirty_index_rebuild.load(Ordering::Relaxed),
            Ordering::Relaxed,
        );
        self.last_recording_mode
            .store(ctx.recording_mode, Ordering::Relaxed);
        self.has_run.store(true, Ordering::Relaxed);
    }

    pub fn record_phase(&self, phase: PerfPhase, elapsed: Duration) {
        let ns = elapsed.as_nanos() as u64;
        let slot = match phase {
            PerfPhase::WorldCallups => &self.cur_callups_ns,
            PerfPhase::WorldNationalMatches => &self.cur_world_matches_ns,
            PerfPhase::ParallelContinents => &self.cur_continents_ns,
            PerfPhase::AiBatch => &self.cur_ai_batch_ns,
            PerfPhase::ResultProcessing => &self.cur_result_proc_ns,
            PerfPhase::ManagerMarket => &self.cur_manager_market_ns,
            PerfPhase::GlobalCompetitions => &self.cur_global_comp_ns,
            PerfPhase::Cleanup => &self.cur_cleanup_ns,
            PerfPhase::Awards => &self.cur_awards_ns,
        };
        slot.fetch_add(ns, Ordering::Relaxed);
    }

    pub fn record_ai_batch_active(&self) {
        self.cur_ai_active.store(true, Ordering::Relaxed);
    }

    /// Whole `FootballEngine::play` wall time, recorded once per
    /// match. Together with `record_play_inner` this drives the
    /// match-engine averages on the dashboard.
    pub fn record_match_total(&self, total: Duration) {
        self.cur_match_sim_ns
            .fetch_add(total.as_nanos() as u64, Ordering::Relaxed);
        self.cur_match_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Recorded once per `play_inner` call. `tick_count` is the number
    /// of `increment_time()` iterations observed for that state.
    pub fn record_play_inner(&self, tick_count: u64, elapsed: Duration) {
        self.cur_match_tick_ns
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
        self.cur_match_tick_count
            .fetch_add(tick_count, Ordering::Relaxed);
    }

    /// Time spent in `FootballEngine::build_result` per match — the
    /// stat-aggregation / rating phase. Surfaces as "Match result
    /// processing" on the dashboard once divided by match count.
    pub fn record_match_result_processing(&self, elapsed: Duration) {
        self.cur_match_result_proc_ns
            .fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    /// Match-storage write happens in the HTTP handler AFTER
    /// `simulate_with` returns, so the per-tick scratch is already
    /// finalised. Write directly into the published snapshot — readers
    /// see the freshest storage timing as soon as the write completes.
    pub fn record_match_storage(&self, elapsed: Duration) {
        self.last_match_storage_ns
            .store(elapsed.as_nanos() as u64, Ordering::Relaxed);
    }

    pub fn mark_dirty_index_rebuild(&self) {
        self.cur_dirty_index_rebuild
            .store(true, Ordering::Relaxed);
    }

    pub fn scope(&'static self, phase: PerfPhase) -> PhaseScope {
        PhaseScope {
            counters: self,
            phase,
            start: Instant::now(),
        }
    }

    pub fn snapshot(&self) -> PerfSnapshot {
        PerfSnapshot {
            has_run: self.has_run.load(Ordering::Relaxed),
            total_ns: self.last_total_ns.load(Ordering::Relaxed),
            was_match_day: self.last_was_match_day.load(Ordering::Relaxed),
            match_day_ns: self.last_match_day_ns.load(Ordering::Relaxed),
            non_match_day_ns: self.last_non_match_day_ns.load(Ordering::Relaxed),
            callups_ns: self.last_callups_ns.load(Ordering::Relaxed),
            world_matches_ns: self.last_world_matches_ns.load(Ordering::Relaxed),
            continents_ns: self.last_continents_ns.load(Ordering::Relaxed),
            ai_batch_ns: self.last_ai_batch_ns.load(Ordering::Relaxed),
            ai_active: self.last_ai_active.load(Ordering::Relaxed),
            result_proc_ns: self.last_result_proc_ns.load(Ordering::Relaxed),
            manager_market_ns: self.last_manager_market_ns.load(Ordering::Relaxed),
            global_comp_ns: self.last_global_comp_ns.load(Ordering::Relaxed),
            cleanup_ns: self.last_cleanup_ns.load(Ordering::Relaxed),
            awards_ns: self.last_awards_ns.load(Ordering::Relaxed),
            match_storage_ns: self.last_match_storage_ns.load(Ordering::Relaxed),
            match_count: self.last_match_count.load(Ordering::Relaxed),
            match_sim_avg_ns: self.last_match_sim_avg_ns.load(Ordering::Relaxed),
            match_tick_avg_ns: self.last_match_tick_avg_ns.load(Ordering::Relaxed),
            match_result_proc_ns: self.last_match_result_proc_ns.load(Ordering::Relaxed),
            simulated_days_total: self.simulated_days_total.load(Ordering::Relaxed),
            matches_simulated_total: self.matches_simulated_total.load(Ordering::Relaxed),
            countries_processed: self.last_countries_processed.load(Ordering::Relaxed),
            leagues_processed: self.last_leagues_processed.load(Ordering::Relaxed),
            clubs_processed: self.last_clubs_processed.load(Ordering::Relaxed),
            players_touched: self.last_players_touched.load(Ordering::Relaxed),
            match_results_written: self.last_match_results_written.load(Ordering::Relaxed),
            panicked_continents: self.last_panicked_continents.load(Ordering::Relaxed),
            dirty_index_rebuild: self.last_dirty_index_rebuild.load(Ordering::Relaxed),
            recording_mode: self.last_recording_mode.load(Ordering::Relaxed),
        }
    }
}

pub struct TickEndContext {
    pub countries: u64,
    pub leagues: u64,
    pub clubs: u64,
    pub players: u64,
    pub match_results_written: u64,
    pub panicked_continents: u32,
    pub recording_mode: bool,
}

pub struct PhaseScope {
    counters: &'static PerfCounters,
    phase: PerfPhase,
    start: Instant,
}

impl Drop for PhaseScope {
    fn drop(&mut self) {
        self.counters.record_phase(self.phase, self.start.elapsed());
    }
}

#[derive(Clone, Debug)]
pub struct PerfSnapshot {
    pub has_run: bool,
    pub total_ns: u64,
    pub was_match_day: bool,
    pub match_day_ns: u64,
    pub non_match_day_ns: u64,

    pub callups_ns: u64,
    pub world_matches_ns: u64,
    pub continents_ns: u64,
    pub ai_batch_ns: u64,
    pub ai_active: bool,
    pub result_proc_ns: u64,
    pub manager_market_ns: u64,
    pub global_comp_ns: u64,
    pub cleanup_ns: u64,
    pub awards_ns: u64,
    pub match_storage_ns: u64,

    pub match_count: u64,
    pub match_sim_avg_ns: u64,
    pub match_tick_avg_ns: u64,
    pub match_result_proc_ns: u64,

    pub simulated_days_total: u64,
    pub matches_simulated_total: u64,

    pub countries_processed: u64,
    pub leagues_processed: u64,
    pub clubs_processed: u64,
    pub players_touched: u64,
    pub match_results_written: u64,
    pub panicked_continents: u64,
    pub dirty_index_rebuild: bool,
    pub recording_mode: bool,
}
