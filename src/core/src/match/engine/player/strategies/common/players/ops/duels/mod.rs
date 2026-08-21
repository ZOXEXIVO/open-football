//! **One-on-one contests** — the micro-resolvers where a single
//! player's technique is contested by a single opponent.
//!
//! * [`dribble_duel`] — carrier vs defender: beat him, lose it, or win
//!   a foul.
//! * [`marker_evasion`] — the attacking half of man-marking: blind
//!   side, double movement, drop-off.
//! * [`first_touch`] — winning the reception: cushion it, or take a
//!   heavy touch and let the marker in.

pub mod dribble_duel;
pub mod first_touch;
pub mod marker_evasion;

pub use dribble_duel::*;
pub use first_touch::*;
pub use marker_evasion::*;
