mod awards;
mod country_info;
mod data;
mod loan_wages;
mod result;
mod seeding;

pub use country_info::CountryInfo;
pub use data::SimulatorData;
pub use result::{SimulationResult, WorldWorkloadCounts};

use crate::ai::{AiBatchProcessor, PendingAiRequest};
use crate::club::ai::apply_ai_responses;
use crate::club::board::manager_market;
use crate::competitions::simulation::GlobalCompetitionSimulator;
use crate::config::SimulatorConfig;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::ContinentResult;
use crate::continent::national::world as national_world;
use crate::country::result::transfers::{GlobalFreeAgentSummary, snapshot_global_free_agents};
use crate::performance::{
    PerfCounters, PerfPhase, ResultProcBreakdown, ResultProcSubphase, TickEndContext,
};
use crate::transfers::pipeline::{PipelineProcessor, PlayerSummary};
use awards::{
    MondayAwardCache, MonthlyAwardsTick, SeasonAwardsTick, TeamOfTheWeekTick, TeamOfTheYearTick,
    WeeklyAwardsTick, WorldPlayerOfYearTick, YoungTeamOfTheWeekTick, YoungWeeklyAwardsTick,
};
use chrono::{Datelike, Duration, Weekday};
use rayon::prelude::*;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};

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
        let breakdown = ResultProcBreakdown::instance();
        perf.begin_tick();
        // Per-tick scratch for the Phase C / Cleanup subphase breakdown.
        // The aggregate parent counter (`PerfPhase::ResultProcessing`)
        // already brackets the whole serial loop; this breakdown lets
        // the dashboard / logs attribute time to specific functions
        // inside it so we can refactor the hot one with confidence.
        breakdown.reset();

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
        // Build the read-only world snapshot once, before the parallel
        // pass starts. Each worker thread gets a Copy of the struct
        // (it's only references inside) so the borrow checker sees
        // distinct shared borrows of `data.country_info`, `data.indexes`,
        // and the freshly-built `world_pool` / `global_free_agents`
        // snapshots, in parallel with the `&mut data.continents` from
        // `par_iter_mut`. Different fields ⇒ split borrow ⇒ safe.
        let world_date = data.date;
        let pool_date = data.date.date();
        let world_pool: Vec<PlayerSummary> = {
            let _g = breakdown.scope(ResultProcSubphase::WorldSnapshots);
            data.continents
                .iter()
                .flat_map(|cont| &cont.countries)
                .flat_map(|c| PipelineProcessor::collect_player_pool(c, pool_date))
                .collect()
        };
        let global_fa_snapshot: Vec<GlobalFreeAgentSummary> = {
            let _g = breakdown.scope(ResultProcSubphase::WorldSnapshots);
            snapshot_global_free_agents(data, pool_date)
        };
        let world_country_info = &data.country_info;
        let world_indexes = data.indexes.as_ref();
        let world = crate::league::result::WorldSnapshot {
            date: world_date,
            country_info: world_country_info,
            indexes: world_indexes,
            world_pool: &world_pool,
            global_free_agents: &global_fa_snapshot,
        };
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
                        continent.simulate(ctx_ref.with_continent(cid), world)
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

        // Phase C: drain Phase-A's deferred ops against post-AI data.
        // World snapshots were built before Phase A so the parallel pass
        // could read them; we expose the same view here via the
        // `daily_*` caches so any legacy callers (test harnesses,
        // continental-cup paths) still find them. Cleared at the end of
        // the phase so the next tick rebuilds.
        data.daily_world_player_pool = Some(world_pool);
        data.daily_global_free_agents = Some(global_fa_snapshot);
        {
            let _g = perf.scope(PerfPhase::ResultProcessing);

            for continent_result in results {
                continent_result.process(data, &mut result);
            }
        }
        data.daily_world_player_pool = None;
        data.daily_global_free_agents = None;

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
            loan_wages::settle_parent_residual_loan_wages(data);
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

        // Log the per-subphase breakdown of Phase C work. One line per
        // tick at info!, sorted by elapsed time descending so the hot
        // function shows first. Diagnostic only — drop or gate behind a
        // setting once the refactor is done.
        let mut rows = breakdown.snapshot();
        let total_ns: u64 = rows.iter().map(|(_, ns)| *ns).sum();
        if total_ns > 0 {
            rows.sort_by(|a, b| b.1.cmp(&a.1));
            let parts: Vec<String> = rows
                .iter()
                .filter(|(_, ns)| *ns > 0)
                .map(|(name, ns)| {
                    let pct = (*ns as f64) * 100.0 / (total_ns as f64);
                    format!("{}={:.1}ms({:.1}%)", name, (*ns as f64) / 1.0e6, pct)
                })
                .collect();
            log::info!(
                "phase_c_breakdown total={:.1}ms {}",
                (total_ns as f64) / 1.0e6,
                parts.join(" ")
            );
        }

        result
    }
}
