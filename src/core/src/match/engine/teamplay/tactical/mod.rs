//! Team-level tactical state shared by all eleven players on a side.
//!
//! The per-player state machines (defenders, midfielders, forwards, GK)
//! used to evaluate ball/opponent proximity from scratch every tick and
//! produced "eleven independent agents" behaviour. Real football runs
//! off team-level phases — build-up, progression, attack, transition,
//! settled defence — that every player reads and respects. This module
//! defines that shared layer; it is recomputed periodically from the
//! ball/possession/time signals and consulted by player states when
//! they make branching decisions.
//!
//! All compute helpers are associated functions on `TeamTacticalState`
//! (or `BallZone` / `BallSideZone`) — there are no loose helper
//! functions in this module. Keeping the math attached to the type
//! makes the calculator boundary obvious from a `cargo doc` view.
//!
//! Split by what each part answers:
//!
//! | Module | Concern |
//! |---|---|
//! | [`phase`] | How the game is read: [`GamePhase`], [`BallZone`], [`BallSideZone`], and the phase / transition-window math |
//! | [`inputs`] | What `refresh` is handed: [`TacticalRefreshInputs`] and [`TeamSkillAggregates`] |
//! | [`team_state`] | [`TeamTacticalState`] itself and the `refresh` pass that fills it |
//! | [`signals`] | The tactic / score / clock math behind every scalar on it |
//! | [`quality`] | The team-quality corrections `refresh` lays on top of those signals |
//!
//! The math stays attached to `TeamTacticalState` — a type may carry
//! several `impl` blocks, so each concern's functions now sit in the file
//! with the tests that pin them. Everything this module exported before
//! the split is re-exported below, so every `tactical::Item` path in the
//! engine keeps resolving.

pub mod inputs;
pub mod phase;
pub mod quality;
pub mod signals;
pub mod team_state;

pub use inputs::{TacticalRefreshInputs, TeamSkillAggregates};
pub use phase::{BallSideZone, BallZone, GamePhase};
pub use team_state::TeamTacticalState;
