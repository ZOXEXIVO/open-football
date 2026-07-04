//! Root-level matchday result.
//!
//! `WorldMatchdayResult` is the single value the simulator carries
//! through one matchday tick. The per-continent `Continent::simulate`
//! pass ONLY builds `Match::make` objects and adds its
//! `ContinentBuildOutput` here — no engine dispatch happens during
//! "simulate". The dispatch is then performed once globally by
//! `WorldMatchdayResult::process`, which aggregates every continent's
//! matches into ONE batch, calls `MatchRuntime::engine_pool().play(..)`
//! exactly once, and fans the results back through each continent's
//! post-match pass.
//!
//! Why this layering: the previous shape had `engine_pool().play` per
//! continent — small continents dispatched half-empty batches, and
//! the worker fleet was fanned out one continent at a time. With one
//! global batch the DistributedDispatcher round-robins the entire
//! world's matches across every worker simultaneously, so the fleet
//! stays saturated through the whole matchday.
//!
//! Lifecycle in `FootballSimulator::simulate_with`:
//!   1. Per-continent parallel `Continent::simulate` → fills
//!      `WorldMatchdayResult::builds` with `Option<ContinentBuildOutput>`
//!      (None for build panics).
//!   2. `WorldMatchdayResult::process(continents, world)` — root
//!      dispatch + per-continent fan-out, populates
//!      `self.continents` with `Vec<ContinentResult>`.
//!   3. `collect_domestic_signed_ids` feeds the world-wide
//!      transfer-interest cleanup sweep.
//!   4. `drain_into` consumes the wrapper and routes every
//!      `ContinentResult::process` into `data` and the tick's
//!      `SimulationResult`.

use std::ops::Range;
use std::panic::{self, AssertUnwindSafe};

use log::info;
use rayon::iter::{IndexedParallelIterator, IntoParallelIterator, ParallelIterator};
use rayon::prelude::IntoParallelRefMutIterator;

use crate::MatchRuntime;
use crate::SimulationResult;
use crate::SimulatorData;
use crate::continent::{Continent, ContinentBuildOutput, ContinentBuildState, ContinentResult};
use crate::country::result::transfers::FreeAgentBumpBatch;
use crate::league::result::WorldSnapshot;
use crate::r#match::{Match, MatchResult};

use super::{ContinentPanicMetrics, panic_message};

#[derive(Default)]
pub struct WorldMatchdayResult<'gc> {
    /// Per-continent build outputs in `data.continents` order.
    /// Populated by the simulate phase; drained by `process`. `None`
    /// for continents whose build panicked.
    pub builds: Vec<Option<ContinentBuildOutput<'gc>>>,
    /// Per-continent results. Empty before `process`; one entry per
    /// continent (in `data.continents` order) afterwards.
    pub continents: Vec<ContinentResult>,
    /// Number of matches dispatched in this tick's GLOBAL engine_pool
    /// batch. `0` before `process` runs, and on idle days.
    pub dispatched: usize,
}

impl<'gc> WorldMatchdayResult<'gc> {
    pub fn new() -> Self {
        WorldMatchdayResult {
            builds: Vec::new(),
            continents: Vec::new(),
            dispatched: 0,
        }
    }

    /// Construct from a pre-collected build list. Used by the
    /// simulator after its per-continent parallel `simulate` pass.
    pub fn from_builds(builds: Vec<Option<ContinentBuildOutput<'gc>>>) -> Self {
        WorldMatchdayResult {
            builds,
            continents: Vec::new(),
            dispatched: 0,
        }
    }

    /// Append a single continent's build output. Available so non-
    /// parallel callers (tests, ad-hoc tools) can drive the same
    /// pipeline one continent at a time.
    pub fn add(&mut self, output: ContinentBuildOutput<'gc>) {
        self.builds.push(Some(output));
    }

    /// Record a panicked continent so the index alignment with
    /// `data.continents` is preserved through `process`.
    pub fn add_panicked(&mut self) {
        self.builds.push(None);
    }

    /// Total `Match::make` count across every continent's build —
    /// the size of the single global batch `process` will dispatch.
    pub fn match_total(&self) -> usize {
        self.builds.iter().flatten().map(|b| b.matches.len()).sum()
    }

    /// Root-level dispatch + per-continent process. Aggregates EVERY
    /// continent's `Match::make` outputs into ONE global `Vec<Match>`,
    /// calls `MatchRuntime::engine_pool().play(..)` exactly once
    /// (so the DistributedDispatcher round-robins the whole tick's
    /// matchday across the worker fleet), then fans the resulting
    /// `Vec<MatchResult>` back through each continent's post-match
    /// pass in parallel.
    ///
    /// After this returns, `self.builds` is empty (its matches and
    /// resume state were consumed) and `self.continents` holds one
    /// `ContinentResult` per slot in build order — the caller drains
    /// AI requests, runs the periodic sub-phases, batches the
    /// cross-country signing-interest cleanup, and finally calls
    /// `drain_into` to fan results into `data` + the tick result.
    ///
    /// `continents` is `&mut data.continents` — taken as a split
    /// borrow alongside the immutable `world` snapshot.
    pub fn process(&mut self, continents: &mut [Continent], world: WorldSnapshot<'_>) {
        let builds = std::mem::take(&mut self.builds);

        // 1. Flatten every continent's matches into one global Vec
        //    while remembering each continent's slice as a `Range`
        //    so we can split results back without re-grouping by id.
        let mut global_matches: Vec<Match> = Vec::new();
        let mut continent_ranges: Vec<Range<usize>> = Vec::with_capacity(builds.len());
        let mut build_states: Vec<Option<ContinentBuildState<'gc>>> =
            Vec::with_capacity(builds.len());
        for build in builds {
            let start = global_matches.len();
            match build {
                Some(ContinentBuildOutput { matches, state, .. }) => {
                    global_matches.extend(matches);
                    continent_ranges.push(start..global_matches.len());
                    build_states.push(Some(state));
                }
                None => {
                    continent_ranges.push(start..start);
                    build_states.push(None);
                }
            }
        }

        // 2. ONE global engine_pool dispatch. The DistributedDispatcher
        //    sees a single batch covering the whole world's matchday
        //    and round-robins it across every connected worker —
        //    no per-continent fan-out, no half-empty batches.
        self.dispatched = global_matches.len();
        let global_results: Vec<MatchResult> = if self.dispatched == 0 {
            Vec::new()
        } else {
            info!(
                "world matchday: dispatching {} matches in one global batch",
                self.dispatched
            );
            MatchRuntime::engine_pool().play(global_matches)
        };

        // 3. Slice results back per continent using the ranges we
        //    captured before the dispatch. `to_vec` copies the slice
        //    so each continent's process pass owns its share.
        let per_continent: Vec<Vec<MatchResult>> = continent_ranges
            .iter()
            .map(|r| global_results[r.clone()].to_vec())
            .collect();

        // 4. Parallel fan-out across continents. Each continent
        //    routes its slice back through its countries (which fan
        //    out parallel inside each country). A panic here is
        //    isolated per continent — recorded on the global counter
        //    and substituted with an empty result, same shape as the
        //    simulate-side guard.
        let results: Vec<ContinentResult> = continents
            .par_iter_mut()
            .zip(build_states.into_par_iter())
            .zip(per_continent.into_par_iter())
            .map(|((continent, state_opt), results)| {
                let cid = continent.id;
                let name = continent.name.clone();
                match state_opt {
                    None => ContinentResult::new(cid, Vec::new(), Vec::new()),
                    Some(state) => panic::catch_unwind(AssertUnwindSafe(|| {
                        continent.process_results(world, state, results)
                    }))
                    .unwrap_or_else(|payload| {
                        ContinentPanicMetrics::record();
                        let msg = panic_message(&payload);
                        log::error!(
                            "event=continent_process_panic continent_id={} continent_name={:?} message={:?} tick_action=continue_with_empty_result",
                            cid, name, msg
                        );
                        ContinentResult::new(cid, Vec::new(), Vec::new())
                    }),
                }
            })
            .collect();

        self.continents = results;
    }

    /// Collect every domestically-signed player id staged on each
    /// country's `DeferredTransferOps`. Fed to the world-wide
    /// `cleanup_player_transfer_interest_batch` sweep so shortlists
    /// in OTHER countries can be pruned in one pass before the per-
    /// continent drain commits the signings.
    pub fn collect_domestic_signed_ids(&self) -> Vec<u32> {
        self.continents
            .iter()
            .flat_map(|cr| cr.countries.iter())
            .filter_map(|country_r| country_r.deferred_transfer_ops.as_ref())
            .flat_map(|ops| ops.domestic_signed_ids.iter().copied())
            .collect()
    }

    /// Aggregate every country's free-agent market bumps (offer / reject
    /// / block-reason) into one [`FreeAgentBumpBatch`]. Fed to the
    /// world-wide `apply_free_agent_market_bumps_batch` pass so the
    /// `data.free_agents` pool is walked ONCE per tick instead of once
    /// per country (the per-country drain was `O(countries × pool)`).
    pub fn collect_free_agent_bumps(&self) -> FreeAgentBumpBatch {
        let mut batch = FreeAgentBumpBatch::default();
        for ops in self
            .continents
            .iter()
            .flat_map(|cr| cr.countries.iter())
            .filter_map(|country_r| country_r.deferred_transfer_ops.as_ref())
        {
            batch
                .offered_ids
                .extend(ops.global_offered_ids.iter().copied());
            batch
                .rejected_ids
                .extend(ops.global_rejected_ids.iter().copied());
            batch
                .block_reasons
                .extend(ops.global_block_reasons.iter().copied());
        }
        batch
    }

    /// Country ids whose season just rolled over this tick (any of their
    /// leagues flagged `new_season_started`). Fed to the parallel
    /// season-start career-history snapshot so every just-ended-season
    /// country is snapshotted in one fan-out before the serial drain
    /// runs the cross-country loan returns.
    pub fn collect_new_season_country_ids(&self) -> Vec<u32> {
        self.continents
            .iter()
            .flat_map(|cr| cr.countries.iter())
            .filter(|country_r| country_r.leagues.iter().any(|l| l.new_season_started))
            .map(|country_r| country_r.country_id)
            .collect()
    }

    /// Final drain. Consumes `self` and routes every continent's
    /// post-match work back into `data` and the tick's
    /// `SimulationResult` (match storage, transfer execution, league
    /// result push, continental cup matchday, etc.). After this
    /// returns there is no per-continent matchday state left for the
    /// rest of the tick.
    pub fn drain_into(self, data: &mut SimulatorData, result: &mut SimulationResult) {
        for continent_result in self.continents {
            continent_result.process(data, result);
        }
    }
}
