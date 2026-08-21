//! **Instrumentation.** Compiled only under `match-logs`, inert until
//! armed, and read by the `dev_match` harness — never delete these.
//!
//! * [`flight_diag`] — where the ball actually goes each tick, and which
//!   line of code sent it there. "The ball jumped" is not actionable;
//!   "the ball jumped 91u inside `try_block_shot`" is.
//! * [`teleport`] — the whole-tick relocation census. `flight_diag` only
//!   sees `Ball::update`; this also sees the resolvers and the player
//!   layer that run after it, which is where the set pieces live.
//! * [`frame_trace`] — the woodwork's own per-tick ball trace, captured
//!   around a hit.
//! * [`assist_diag`] — why a goal did or didn't carry an assist.

#[cfg(feature = "match-logs")]
pub mod assist_diag;
#[cfg(feature = "match-logs")]
pub mod flight_diag;
#[cfg(feature = "match-logs")]
pub mod frame_trace;
#[cfg(feature = "match-logs")]
pub mod teleport;
