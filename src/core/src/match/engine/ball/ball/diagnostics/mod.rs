//! **Instrumentation.** Compiled only under `match-logs`, inert until
//! armed, and read by the `dev_match` harness — never delete these.
//!
//! * [`block_diag`] — why shot blocks do not happen: the counter alone
//!   cannot say whether the shot never reaches the check, no defender is
//!   ever in the lane, or the roll simply fails.
//! * [`flight_diag`] — where the ball actually goes each tick, and which
//!   line of code sent it there. "The ball jumped" is not actionable;
//!   "the ball jumped 91u inside `try_block_shot`" is.
//! * [`teleport`] — the whole-tick relocation census. `flight_diag` only
//!   sees `Ball::update`; this also sees the resolvers and the player
//!   layer that run after it, which is where the set pieces live.
//! * [`frame_trace`] — the woodwork's own per-tick ball trace, captured
//!   around a hit.
//! * [`assist_diag`] — why a goal did or didn't carry an assist.
//! * [`knock_diag`] — the keeper knock-chain census: every loose contact
//!   off a goalkeeper, linked into chains, because the reported defect is
//!   never the first touch.
//! * [`strike_diag`] — the VERTICAL axis of a grant and of a strike:
//!   who is given a ball at what height, who kicks one at what height,
//!   and what a refused strike costs before it is struck for real.
//! * [`lane_diag`] — how close the nearest opponent came to each pass
//!   in flight, and whether it got through him anyway.
//! * [`arrival_diag`] — what a grant does to the ball's motion: pace in
//!   and out, the gap it is taken at, and whether the step after it is a
//!   reversal or a write.
//! * [`goal_kick_diag`] — where a goal kick's wait goes, what the keeper
//!   does while it waits, and how it is put back into play.

#[cfg(feature = "match-logs")]
pub mod arrival_diag;
#[cfg(feature = "match-logs")]
pub mod assist_diag;
#[cfg(feature = "match-logs")]
pub mod block_diag;
#[cfg(feature = "match-logs")]
pub mod flight_diag;
#[cfg(feature = "match-logs")]
pub mod frame_trace;
#[cfg(feature = "match-logs")]
pub mod goal_kick_diag;
#[cfg(feature = "match-logs")]
pub mod knock_diag;
#[cfg(feature = "match-logs")]
pub mod lane_diag;
#[cfg(feature = "match-logs")]
pub mod strike_diag;
#[cfg(feature = "match-logs")]
pub mod teleport;
