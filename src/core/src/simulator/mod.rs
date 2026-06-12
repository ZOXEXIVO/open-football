mod awards;
mod country_info;
mod data;
mod loan_wages;
mod matchday;
mod result;
mod seeding;

pub use country_info::CountryInfo;
pub use data::SimulatorData;
pub use matchday::WorldMatchdayResult;
pub use result::{SimulationResult, WorldWorkloadCounts};

use crate::MatchRuntime;
use crate::ai::AiBatchProcessor;
use crate::club::ai::apply_ai_responses;
use crate::club::board::manager_market;
use crate::competitions::simulation::GlobalCompetitionSimulator;
use crate::config::SimulatorConfig;
use crate::context::{GlobalContext, SimulationContext};
use crate::continent::ContinentAwardOutcome;
use crate::continent::ContinentBuildOutput;
use crate::continent::ContinentResult;
use crate::continent::national::world as national_world;
use crate::country::result::transfers::free_agent_audit::FreeAgentMarketAuditor;
use crate::country::result::transfers::{GlobalFreeAgentSummary, snapshot_global_free_agents};
use crate::league::result::WorldSnapshot;
use crate::performance::{PerfCounters, PerfPhase, TickEndContext};
use crate::transfers::pipeline::{PipelineProcessor, PlayerSummary};
use crate::utils::DateUtils;
use awards::{
    MondayAwardCache, MonthlyAwardsTick, SeasonAwardsTick, TeamOfTheWeekTick, TeamOfTheYearTick,
    WeeklyAwardsTick, WorldPlayerOfYearTick, YoungTeamOfTheWeekTick, YoungWeeklyAwardsTick,
};
use chrono::{Datelike, Duration, Weekday};
use rayon::prelude::*;
use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};

fn panic_message(payload: &(dyn Any + Send)) -> &'static str {
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
/// `ContinentPanicMetrics::total()`.
static PANICKED_CONTINENTS: AtomicU64 = AtomicU64::new(0);

/// Process-global accessor for the swallowed-continent-panic counter.
pub struct ContinentPanicMetrics;

impl ContinentPanicMetrics {
    /// Total continent panics swallowed since process start.
    pub fn total() -> u64 {
        PANICKED_CONTINENTS.load(Ordering::Relaxed)
    }

    /// Record one swallowed continent panic.
    pub fn record() {
        PANICKED_CONTINENTS.fetch_add(1, Ordering::Relaxed);
    }
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

        // Phase A: matchday simulation in two clearly separated halves.
        //
        //   A1 — parallel BUILD across continents. Each call to
        //        `Continent::simulate` ONLY produces `Match::make`
        //        objects and adds its `ContinentBuildOutput` to the
        //        per-tick `WorldMatchdayResult`. No engine dispatch
        //        happens during simulate.
        //   A2 — `WorldMatchdayResult::process` is the ROOT-LEVEL
        //        accumulator. It flattens every continent's matches
        //        into one collection, calls
        //        `MatchRuntime::engine_pool().play(..)` exactly once,
        //        and fans the results back through each continent's
        //        post-match pass (parallel across continents). The
        //        DistributedDispatcher sees a single global batch
        //        spanning the entire world — workers stay saturated
        //        for the whole matchday instead of being fanned out
        //        once per continent (small continents used to
        //        dispatch half-empty batches; big ones used to pin
        //        slow workers as the matchday's tail latency).
        //
        // A panic inside one continent must not kill the whole tick —
        // a single buggy state machine or malformed save row would
        // otherwise unwind the Rayon pool and dump the player's save.
        // `AssertUnwindSafe` is sound here because the closure mutates
        // only its own continent (no shared `&mut` state) and doesn't
        // hold any locks; the Rayon worker doesn't carry poisoned
        // state across iterations. Panic is surfaced via the
        // `PANICKED_CONTINENTS` counter and a structured log line;
        // surviving continents still advance. Per-tick count is
        // recovered as the delta on the atomic since map closures
        // running in parallel can't share a `&mut u32`.
        let panicks_before = ContinentPanicMetrics::total();
        // Build the read-only world snapshot once, before the parallel
        // pass starts. Each worker thread gets a Copy of the struct
        // (it's only references inside) so the borrow checker sees
        // distinct shared borrows of `data.country_info`, `data.indexes`,
        // and the freshly-built `world_pool` / `global_free_agents`
        // snapshots, in parallel with the `&mut data.continents` from
        // `par_iter_mut`. Different fields ⇒ split borrow ⇒ safe.
        let world_date = data.date;
        let pool_date = data.date.date();
        let world_pool: Vec<PlayerSummary> = data
            .continents
            .par_iter()
            .flat_map(|cont| cont.countries.par_iter())
            .flat_map_iter(|c| PipelineProcessor::collect_player_pool(c, pool_date))
            .collect();
        let global_fa_snapshot: Vec<GlobalFreeAgentSummary> =
            snapshot_global_free_agents(data, pool_date);
        let world_country_info = &data.country_info;
        let world_indexes = data.indexes.as_ref();
        let world = WorldSnapshot {
            date: world_date,
            country_info: world_country_info,
            indexes: world_indexes,
            world_pool: &world_pool,
            global_free_agents: &global_fa_snapshot,
        };
        let mut world_matchday: WorldMatchdayResult<'_> = {
            let _g = perf.scope(PerfPhase::ParallelContinents);

            // A1: parallel build. Each `Continent::simulate` returns a
            // `ContinentBuildOutput` carrying its `Match::make`
            // objects and a resume token. A panic substitutes `None`
            // so the slot's index alignment with `data.continents`
            // survives — A2 then skips its dispatch slot and emits an
            // empty `ContinentResult`.
            let builds: Vec<Option<ContinentBuildOutput<'_>>> = data
                .continents
                .par_iter_mut()
                .map(|continent| {
                    let cid = continent.id;
                    let name = continent.name.clone();
                    let ctx_ref = &ctx;
                    match panic::catch_unwind(AssertUnwindSafe(|| {
                        continent.simulate(ctx_ref.with_continent(cid), world)
                    })) {
                        Ok(output) => Some(output),
                        Err(payload) => {
                            ContinentPanicMetrics::record();
                            let msg = panic_message(&payload);
                            log::error!(
                                "event=continent_simulate_panic continent_id={} continent_name={:?} message={:?} tick_action=continue_with_empty_result",
                                cid, name, msg
                            );
                            None
                        }
                    }
                })
                .collect();

            // Wrap every continent's build into the single root-level
            // result. From here on the tick operates on `world_matchday`
            // rather than open-coded Vec<Option<ContinentBuildOutput>>.
            let mut wm = WorldMatchdayResult::from_builds(builds);

            // A2: root-level dispatch + per-continent fan-out. Single
            // `engine_pool().play(..)` call across the entire world.
            wm.process(&mut data.continents, world);
            wm
        };
        result.panicked_continents = (ContinentPanicMetrics::total() - panicks_before) as u32;

        // Phase B: drain AI requests staged on each ContinentResult and
        // batch-execute them. Lock-free — every request travelled up the
        // result chain owned by exactly one worker. The tick waits for
        // the batch to finish — no timeout, no dropped responses.
        let all_requests = world_matchday.drain_ai_requests();
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

            // Continent-local periodic sub-passes — monthly rankings,
            // quarterly economic zone, yearly regulations, year-end
            // awards rank + cup-finals. Each closure mutates only its
            // own continent, so they run in parallel across continents.
            // Pulled out of the serial drain below because they're the
            // four heaviest periodic walks (rankings/economics aggregate
            // every club; the awards walk every player in every team in
            // every league).
            let phase_date = current_date.date();
            let award_outcomes: Vec<ContinentAwardOutcome> = data
                .continents
                .par_iter_mut()
                .filter_map(|continent| {
                    if DateUtils::is_month_beginning(phase_date) {
                        ContinentResult::update_continental_rankings(continent);
                    }
                    if DateUtils::is_quarter_start(phase_date) {
                        ContinentResult::update_economic_zone(continent);
                    }
                    if DateUtils::is_year_start(phase_date) {
                        ContinentResult::update_continental_regulations(continent, phase_date);
                    }
                    if DateUtils::is_year_end(phase_date) {
                        Some(ContinentResult::build_continental_award_outcome(
                            continent, phase_date,
                        ))
                    } else {
                        None
                    }
                })
                .collect();

            // Apply cross-continent player events for the year-end
            // awards. `data.player_mut` resolves against every
            // continent, so this stays serial. Small N (3 nominees +
            // 1 winner per continent per year).
            for outcome in award_outcomes {
                ContinentResult::apply_continental_award_outcome(data, outcome, phase_date);
            }

            // Cross-country interest sweep — batched. Each country's
            // Phase-A free-agent matching stages domestic signings on
            // its `DeferredTransferOps.domestic_signed_ids`; the
            // per-country drain used to fire `cleanup_player_transfer_interest`
            // for each id, re-walking every other country's shortlists
            // once per signing. We aggregate every signed id first,
            // then walk the world once in parallel via
            // `cleanup_player_transfer_interest_batch`.
            let all_signed_ids = world_matchday.collect_domestic_signed_ids();
            PipelineProcessor::cleanup_player_transfer_interest_batch(data, &all_signed_ids);

            world_matchday.drain_into(data, &mut result);
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
            // Monthly check, gated internally on `free_since` >= 12mo,
            // with a deterministic hard bound so unlucky rolls can't
            // strand anyone in the pool for multiple seasons.
            data.process_free_agent_retirements(today);
            // Monthly visibility into the long tail: one debug line per
            // 12-month-plus free agent explaining why they're unsigned
            // (no-op unless debug logging is enabled).
            FreeAgentMarketAuditor::log_long_term(data, today);
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
            recording_mode: MatchRuntime::recordings_mode(),
        });

        result
    }
}
