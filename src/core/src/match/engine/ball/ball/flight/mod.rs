//! **The ball in motion** — what it does between the moment it leaves a
//! boot and the moment somebody or something stops it.
//!
//! * [`ballistics`] — the constants the whole engine's physics is priced
//!   in (gravity, drag, ground friction) and the solver every strike
//!   site uses to pick a launch velocity for a range, an apex, or an
//!   arrival.
//! * [`motion`] — the per-tick integration: drag, gravity, friction,
//!   spin ([`SpinModel`](motion::SpinModel)), owner tracking, and the
//!   boundary inset.
//! * [`aerial`] — the ball above head height: how high a footballer can
//!   reach ([`AerialReach`]), and the decided-contest delivery that is
//!   chosen at the strike and applied on arrival.
//! * [`roll`] — how far a ball rolling on the ground will still travel.

pub mod aerial;
pub mod ballistics;
pub mod motion;
pub mod roll;

pub use aerial::{AerialDelivery, AerialOutcome, AerialReach};
pub use ballistics::{AIR_DRAG_PER_TICK, GRAVITY_PER_TICK, GROUND_FRICTION};
pub use roll::BallRoll;
