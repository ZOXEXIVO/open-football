//! Match-flow scaffolding: the surrounding environment, the playing
//! field, scoring/goal bookkeeping, per-tick context, and the final
//! result.
//!
//! Grouped by what each part is FOR, rather than by the order a tick
//! touches them:
//!
//! | Group           | Concern                                                          |
//! |-----------------|------------------------------------------------------------------|
//! | [`arena`]       | The physical stage: weather and pitch, the field and its reset, the goal frame and the kickoff that follows a ball crossing it |
//! | [`celebration`] | The 45-75 s after the ball goes in — a choreographed cutscene, deliberately outside the state machine and outside the RNG stream |
//! | [`context`]     | The world one match carries around: clock, score, squads, plans, referee, and the seeded RNG that makes a replay a replay |
//! | [`result`]      | What comes out at the whistle: the score, the highlights worth keeping, the per-player stat lines, and the payload the league pipeline consumes |
//! | [`touchline`]   | The ground outside the line: the two benches, and the twelve seconds a substitution stops the match for — a cutscene under the same two rules the celebration keeps |
//!
//! `arena` re-exports its three modules under the names they had before
//! the grouping, so `flow::environment`, `flow::field` and `flow::goal`
//! keep resolving; `context` does the same for `flow::rng`. The engine
//! root globs all four groups and must not need to know this layout.

pub mod arena;
pub mod celebration;
pub mod context;
pub mod result;
pub mod touchline;

pub use arena::{environment, field, goal};
pub use context::rng;
