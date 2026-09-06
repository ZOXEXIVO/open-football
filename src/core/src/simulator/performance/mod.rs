//! Wall/CPU accounting for the world simulator's daily tick.
//!
//! The match engine already has [`PhaseProf`][crate::r#match::PhaseProf] for
//! its tick loop; this is the counterpart one level up — the *simulator
//! graph* that wraps the engine. It answers the one question a wall-clock
//! timer cannot: **how many cores was this phase actually using?**
//!
//! Every region is charged both the wall time it took and the process CPU
//! time burned while it ran, and `cpu / wall` is the average number of busy
//! cores. A phase at 1.0 is serial; a phase at 28 on a 32-core box is
//! saturated. That ratio is the point of the whole module — a phase can be
//! cheap in CPU and still dominate the tick because it runs alone.
//!
//! Three instruments, in increasing resolution:
//!
//! * **Phases** — [`PerformanceProfiler::phase`] and
//!   [`PerformanceProfiler::phase_scope`], for regions on the tick's
//!   driving thread. Wall, CPU, and the average width.
//! * **Stages** — [`PerformanceProfiler::stage`] and
//!   [`PerformanceProfiler::stage_scope`], for regions *inside* the rayon
//!   tree, where a process-CPU delta belongs to the whole pool rather than
//!   to the closure. A stage records the summed time across every worker
//!   and the single slowest item, so `sum / max` is the widest that stage
//!   could ever run.
//! * **Width histogram** — a background thread samples the process cycle
//!   counter and the report buckets each phase's wall time by how many
//!   cores were busy. An average of 16 can be 32 cores throughout or 32 for
//!   half the phase and 2 for the rest; only the second is a straggler tail
//!   worth attacking.
//!
//! Enable with `OF_PERFORMANCE` and run the world-sim harness:
//!
//! ```text
//! OF_PERFORMANCE=1 ./target/release/dev_simulate 60
//! ```
//!
//! Off by default: [`PerformanceProfiler::enabled`] is one relaxed atomic
//! load and the phases are top-level (tens per tick, not millions), so the
//! instrumentation is free in production and needs no Cargo feature. Kept
//! in-tree as a permanent diagnostic, like the engine's phase profiler.

mod clock;
mod profiler;
mod sampler;

pub use profiler::{PerformanceProfiler, PhaseScope, StageScope};
