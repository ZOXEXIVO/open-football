//! One type per phase of the daily tick.
//!
//! The phases are ordered by what each must be able to see, and
//! [`crate::simulator::FootballSimulator::simulate_with`] is the only
//! place that ordering is written down. Each type here is free to
//! parallelise internally; none of them knows what runs before or after.

mod epilogue;
mod honours;
mod matchday;
mod periodic;
mod prologue;
mod world_pass;

pub use epilogue::Epilogue;
pub use honours::Honours;
pub use matchday::{MatchdayOutcome, MatchdayPhase};
pub use periodic::PeriodicPasses;
pub use prologue::Prologue;
pub use world_pass::WorldPass;
