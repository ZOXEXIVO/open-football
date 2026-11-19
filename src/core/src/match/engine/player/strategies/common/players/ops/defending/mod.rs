//! **Defensive shape and pressure.**
//!
//! * [`defensive`] — the line itself: where it sits, who is last man,
//!   who marks whom, cover and help targets, box emergencies.
//! * [`pressure`] — how pressed a player is, and whether to counterpress.
//! * [`clearance`] — when a defender in his own area stops trying to
//!   play and puts it out.

pub mod clearance;
pub mod defensive;
pub mod pressure;

pub use clearance::*;
pub use defensive::*;
pub use pressure::*;
