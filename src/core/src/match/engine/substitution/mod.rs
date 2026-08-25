//! Substitution decisions: when and whom to bring on, and the scoring
//! that ranks candidate changes.
//!
//! Three layers, and they answer different questions:
//!
//! * [`urgency`] — **when** the board goes up. One continuous pressure per
//!   side, read from the scoreline, the legs, the trouble on the pitch and
//!   the manager's own temperament.
//! * [`sub_scoring`] — **who** it should be, once it is going up: the
//!   sub-off / sub-in fit, star protection, tactical need.
//! * [`substitutions`] — the pass that puts the two together and performs
//!   the swap.

pub mod sub_scoring;
pub mod substitutions;
pub mod urgency;
