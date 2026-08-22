//! **Instrumentation.** Nothing here changes what the match does; it
//! only reports on it, and the `dev_match` harness reads the numbers —
//! never delete these.
//!
//! * [`phase_prof`] — env-gated per-phase timing for the tick loop, on
//!   in-tree permanently because it costs one relaxed atomic load when
//!   off.
//! * `census` — the `match-logs` samplers the tick calls: defensive
//!   shape, duel gates, and loose-ball chase geometry.
//! * `teleport_probe` — the per-tick ball relocation probe the driver
//!   threads through its phases.
//!
//! The last two are compiled only under `match-logs`; the profiler is
//! always compiled, which is why it is the only one re-exported to the
//! engine root.

pub mod phase_prof;

#[cfg(feature = "match-logs")]
pub mod census;
#[cfg(feature = "match-logs")]
pub mod teleport_probe;
