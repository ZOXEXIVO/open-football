//! **On-ball attacking play** — where to move, who to pass to, whether
//! to shoot.
//!
//! * [`box_movement`] — what an occupant of a `BoxSlot` does with it:
//!   the staged approach, the zone-working cadence, and the run onto the
//!   delivery.
//! * [`movement`] — space and support: dribbling lanes, gaps in the
//!   defensive line, support-run positions, congestion checks.
//! * [`passing`] — pass-option search and safety scoring.
//! * [`forward_shot_decision`] — the shoot / hold / lay-off decision
//!   itself, plus the striking-range, poise, carry and free-kick models
//!   behind it. Owns the `*_diag` counters `dev_match` reads.
//! * [`shooting`] — the context-bound facade the states call
//!   (`in_shooting_range`, `should_shoot_over_pass`, …).
//! * [`xg`] — chance quality, shared by the pre-shot gate and the
//!   post-hoc stat so both price a chance the same way.

pub mod box_movement;
pub mod forward_shot_decision;
pub mod movement;
pub mod passing;
pub mod shooting;
pub mod xg;

pub use box_movement::*;
pub use forward_shot_decision::*;
pub use movement::*;
pub use passing::*;
pub use shooting::*;
pub use xg::*;
