//! What a fixture is worth to one side: how much its result can still
//! change for that club, on one 0..1 scale whichever competition asks.

mod knockout;
mod table;

pub use knockout::{KnockoutStakes, KnockoutTieReading};
pub use table::{TableLine, TableStakes, TableStanding};
