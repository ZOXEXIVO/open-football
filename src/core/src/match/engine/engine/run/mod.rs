//! **One match, start to finish** — the outer lifecycle that wraps the
//! tick loop. Everything here runs once per match (or once per match
//! state), never once per tick.
//!
//! * [`lifecycle`] — the `play()` / `play_seeded()` / `play_with_config()`
//!   entry points and `play_inner`: the loop that drives
//!   one [`MatchState`](crate::r#match::MatchState) to its end and reports
//!   the stoppage time it accrued.
//! * [`shootout`] — the discrete penalty shootout the lifecycle reaches
//!   for when the states are exhausted and the tie is not.
//! * [`stub`] — the stubbed match the `match-stub` feature plays instead:
//!   a result drawn from the two squads with no simulation behind it.
//! * [`result`] — the post-match pass that assembles the
//!   [`MatchResultRaw`](crate::r#match::MatchResultRaw), picks the
//!   highlights, and dumps the blowout profile under `match-logs`.
//!
//! Apart from the stub, the group is `impl FootballEngine`.

pub mod lifecycle;
pub mod result;
pub mod shootout;
#[cfg(feature = "match-stub")]
pub mod stub;
