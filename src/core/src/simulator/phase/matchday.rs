use crate::context::GlobalContext;
use crate::continent::ContinentBuildOutput;
use crate::country::result::transfers::{GlobalFreeAgentPool, GlobalFreeAgentSummary};
use crate::league::result::WorldSnapshot;
use crate::simulator::{ContinentPanicMetrics, WorldMatchdayResult};
use crate::transfers::pipeline::PlayerSummary;
use crate::transfers::scouting::ScoutingPass;
use crate::utils::PerformanceProfiler;
use crate::world::SimulatorData;
use rayon::prelude::*;
use std::panic::{self, AssertUnwindSafe};

/// The matchday, in two clearly separated halves.
///
/// **Build** is a parallel pass across continents that ONLY produces
/// `Match::make` objects — no engine dispatch happens inside
/// `Continent::simulate`. **Process** is the root-level accumulator:
/// [`WorldMatchdayResult::process`] flattens every continent's matches
/// into one collection, dispatches them in a single
/// `MatchRuntime::engine_pool().play(..)` call, and fans the results back
/// through each continent's post-match pass.
///
/// Why the layering: with one global batch the dispatcher round-robins
/// the entire world's matches across every worker simultaneously, so the
/// fleet stays saturated through the whole matchday. Fanning out once per
/// continent meant small continents dispatched half-empty batches and big
/// ones pinned slow workers as the matchday's tail latency.
///
/// A panic inside one continent must not kill the whole tick — a single
/// buggy state machine or malformed save row would otherwise unwind the
/// Rayon pool and dump the player's save. `AssertUnwindSafe` is sound
/// here because the closure mutates only its own continent (no shared
/// `&mut` state) and holds no locks, so the worker carries no poisoned
/// state across iterations. A failed build substitutes `None`, which
/// keeps the slot's index alignment with `data.continents` and makes the
/// process half emit an empty `ContinentResult` for it.
pub struct MatchdayPhase;

/// What the matchday leaves behind for the rest of the tick: the
/// per-continent results to drain, and the two world snapshots the
/// parallel pass read — parked so the periodic passes can republish them
/// on `data` without rebuilding.
pub struct MatchdayOutcome<'gc> {
    pub matchday: WorldMatchdayResult<'gc>,
    pub world_pool: Vec<PlayerSummary>,
    pub free_agents: Vec<GlobalFreeAgentSummary>,
    pub panicked_continents: u32,
}

impl MatchdayPhase {
    pub fn run<'gc>(data: &mut SimulatorData, ctx: &GlobalContext<'gc>) -> MatchdayOutcome<'gc> {
        let panicks_before = ContinentPanicMetrics::total();

        let phase = PerformanceProfiler::phase_scope("A0_world_snapshots", 0);
        let today = data.date.date();
        let world_pool: Vec<PlayerSummary> = data
            .continents
            .par_iter()
            .flat_map(|continent| continent.countries.par_iter())
            .flat_map_iter(|country| ScoutingPass::collect_player_pool(country, today))
            .collect();
        let free_agents = GlobalFreeAgentPool::snapshot(data, today);
        // At the window boundaries the wage world has moved enough that
        // the money axis of `import_capacity` is stale — the Gulf got
        // richer, a league collapsed. The corridor cards are shipped data
        // and never change; only the facts around them do, so this is the
        // whole of the refresh and it runs twice a year.
        if SimulatorData::is_nationality_reseed_day(today) {
            data.rebuild_market_map();
        }
        drop(phase);

        // Each worker thread gets a Copy of the snapshot (it holds only
        // references) so the borrow checker sees distinct shared borrows
        // of `data.country_info`, `data.indexes` and `data.market_map`
        // alongside the `&mut data.continents` from `par_iter_mut`.
        // Different fields ⇒ split borrow ⇒ safe, which is also why the
        // struct is built here rather than behind a constructor.
        let world = WorldSnapshot {
            date: data.date,
            country_info: &data.country_info,
            indexes: data.indexes.as_ref(),
            world_pool: &world_pool,
            global_free_agents: &free_agents,
            market_map: &data.market_map,
        };

        let phase = PerformanceProfiler::phase_scope("A1_build", 0);
        let builds: Vec<Option<ContinentBuildOutput<'gc>>> = data
            .continents
            .par_iter_mut()
            .map(|continent| {
                let id = continent.id;
                let name = continent.name.clone();
                panic::catch_unwind(AssertUnwindSafe(|| {
                    continent.simulate(ctx.with_continent(id), world)
                }))
                .map_err(|payload| ContinentPanicMetrics::swallow(&payload, "simulate", id, &name))
                .ok()
            })
            .collect();
        drop(phase);

        let mut matchday = WorldMatchdayResult::from_builds(builds);

        let phase = PerformanceProfiler::phase_scope("A2_process", 0);
        matchday.process(&mut data.continents, world);
        drop(phase);

        MatchdayOutcome {
            matchday,
            world_pool,
            free_agents,
            panicked_continents: (ContinentPanicMetrics::total() - panicks_before) as u32,
        }
    }
}
