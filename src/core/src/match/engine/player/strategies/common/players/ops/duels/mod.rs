//! **One-on-one contests** — the micro-resolvers where a single
//! player's technique is contested by a single opponent.
//!
//! * [`dribble_duel`] — carrier vs defender: beat him, lose it, or win
//!   a foul.
//! * [`marker_evasion`] — the attacking half of man-marking: blind
//!   side, double movement, drop-off.
//!
//! # The reception is NOT here
//!
//! `first_touch.rs` used to sit alongside these two and it was never
//! wired into a match: its only callers were its own unit tests and
//! `intelligence_tests.rs`. It was load-bearing in the worst way — the
//! miscontrol shortfall was once diagnosed off its deterministic outcome
//! bands, and the diagnosis was wrong about the cause because none of it
//! ran. Deleted 2026-08-26.
//!
//! The live reception model is in two halves, both of them real:
//!
//! * `Ball::roll_first_touch` (`ball/contest/ownership.rs`) — the
//!   skill-rolled touch at the moment a pass arrives, off
//!   `sc::receiving_first_touch`, which decides whether the ball is
//!   killed, runs on, or squirts loose;
//! * `PlayerEventDispatcher::maybe_record_first_touch_loss`
//!   (`player/events/players.rs`) — the stat-line record the rating
//!   helper reads through `touch_quality`.

pub mod dribble_duel;
pub mod marker_evasion;

pub use dribble_duel::*;
pub use marker_evasion::*;
