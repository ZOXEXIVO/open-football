//! The daily tick.
//!
//! [`FootballSimulator::simulate_with`] advances the world by one day. It
//! owns nothing but the ORDER of the phases in `phase` — every piece of
//! work belongs to the module whose domain it is, and the state it all
//! runs against lives in [`crate::world`].
//!
//! The order is load-bearing and each step below says what it must see:
//!
//! * **0 `Prologue`** — the tables and call-ups every continent reads.
//! * **A `MatchdayPhase`** — parallel build across continents, then ONE
//!   global engine dispatch, then the per-continent fan-out.
//! * **C `PeriodicPasses`** — the calendar's own work, the batched
//!   cross-country sweeps, and the drain of everything the matchday
//!   deferred.
//! * **D `WorldPass`** — what only the world level can settle.
//! * **E `Epilogue`** — releasing flags and refreshing derived state.
//! * **F `Honours`** — awards, then the papers that report them.
//!
//! The phases are crate-private on purpose: running one out of order, or
//! on its own, is never correct.

mod matchday;
mod panic;
pub(crate) mod phase;
mod result;

pub use matchday::WorldMatchdayResult;
pub use panic::ContinentPanicMetrics;
pub use result::SimulationResult;

use crate::config::SimulatorConfig;
use crate::utils::PerformanceProfiler;
use crate::world::SimulatorData;
use phase::{Epilogue, Honours, MatchdayPhase, PeriodicPasses, Prologue, WorldPass};

pub struct FootballSimulator;

impl FootballSimulator {
    /// Tick the simulator one day with default tunables. Use
    /// [`simulate_with`](Self::simulate_with) to plumb a
    /// [`SimulatorConfig`] (per-save overrides, faster timeouts in tests).
    pub async fn simulate(data: &mut SimulatorData) -> SimulationResult {
        Self::simulate_with(data, &SimulatorConfig::default()).await
    }

    pub async fn simulate_with(
        data: &mut SimulatorData,
        config: &SimulatorConfig,
    ) -> SimulationResult {
        PerformanceProfiler::init_from_env();
        let mut result = SimulationResult::new();

        let ctx = Prologue::run(data, &mut result);

        let matchday = MatchdayPhase::run(data, &ctx);
        result.panicked_continents = matchday.panicked_continents;

        PeriodicPasses::run(data, matchday, &mut result);
        WorldPass::run(data);
        Epilogue::run(data, config);
        Honours::run(data);

        data.next_date();

        result
    }
}
