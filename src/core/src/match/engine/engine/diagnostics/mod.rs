//! **Instrumentation.** Nothing here changes what the match does; it
//! only reports on it, and the `dev_match` harness reads the numbers —
//! never delete these.
//!
//! * [`phase_prof`] — env-gated per-phase timing for the tick loop, on
//!   in-tree permanently because it costs one relaxed atomic load when
//!   off.
//! * `census` — the `match-logs` samplers the tick calls: defensive
//!   shape, duel gates, and loose-ball chase geometry.
//! * `spacing` — how much room the twenty-two give each other, and what
//!   the largest clump on the pitch is made of.
//! * `heatmap` — the thermal map: where each slot spends its time, both
//!   sides folded into one frame and split by possession phase.
//! * `teleport_probe` — the per-tick ball relocation probe the driver
//!   threads through its phases.
//! * [`state_counters`] — the tackle-flow, shot-gate and stuck-exit
//!   counters the role states bump.
//!
//! The profiler and the state counters are always compiled (the counters'
//! call sites are in the states); the rest only under `match-logs`.

pub mod phase_prof;
pub mod state_counters;

#[cfg(feature = "match-logs")]
pub mod census;
#[cfg(feature = "match-logs")]
pub mod heatmap;
#[cfg(feature = "match-logs")]
pub mod spacing;
#[cfg(feature = "match-logs")]
pub mod teleport_probe;
