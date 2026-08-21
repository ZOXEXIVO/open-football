//! **Defensive shape and pressure.**
//!
//! * [`defensive`] — the line itself: where it sits, who is last man,
//!   who marks whom, cover and help targets, box emergencies.
//! * [`pressure`] — how pressed a player is, and whether to counterpress.
//! * [`panic`] — what being pressed does to his decisions: the panic
//!   clear, shrinking decision time, and clearance quality.

pub mod defensive;
pub mod panic;
pub mod pressure;

pub use defensive::*;
pub use panic::*;
pub use pressure::*;
