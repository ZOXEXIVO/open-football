//! **One match, start to finish** — the outer lifecycle that wraps the
//! tick loop. Everything here runs once per match (or once per match
//! state), never once per tick.
//!
//! * [`lifecycle`] — the `play()` / `play_seeded()` / `play_with_config()`
//!   entry points, the stub match, and `play_inner`: the loop that drives
//!   one [`MatchState`](crate::r#match::MatchState) to its end and reports
//!   the stoppage time it accrued.
//! * [`shootout`] — the discrete penalty shootout the lifecycle reaches
//!   for when the states are exhausted and the tie is not.
//! * [`result`] — the post-match pass that assembles the
//!   [`MatchResultRaw`](crate::r#match::MatchResultRaw), picks the
//!   highlights, and dumps the blowout profile under `match-logs`.
//!
//! The whole group is `impl FootballEngine` — no types of its own.

pub mod lifecycle;
pub mod result;
pub mod shootout;
