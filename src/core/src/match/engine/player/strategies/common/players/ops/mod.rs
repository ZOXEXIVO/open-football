//! Player operations — the shared helper layer the state machines read.
//!
//! Grouped by concern:
//!
//! * [`skill`] — attribute → ability. What a player can do right now,
//!   after fatigue, match standard and trait bias. Knows nothing about
//!   the pitch.
//! * [`attacking`] — on-ball attacking play: movement, passing, and the
//!   shoot-or-not decision with its xG model.
//! * [`defending`] — defensive shape, marking, pressing, and the
//!   under-pressure panic modifier.
//! * [`duels`] — the contested one-on-one micro-resolvers.
//!
//! Each group re-exports its own modules, and this module re-exports
//! the groups, so both `ops::<module>::Item` and the flat `ops::Item`
//! paths keep resolving exactly as they did before the regrouping.

pub mod attacking;
pub mod defending;
pub mod duels;
pub mod skill;

pub use attacking::*;
pub use defending::*;
pub use duels::*;
pub use skill::*;
