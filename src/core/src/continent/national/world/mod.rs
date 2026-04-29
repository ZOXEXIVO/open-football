//! World-aware national-team match pipeline.
//!
//! Per-continent national-team logic silently lost any squad member
//! playing at a foreign club, and split the post-match write path
//! between continental qualifiers and global tournaments. This module
//! is the world-scope counterpart of
//! [`super::national_team_competition`]: every helper here operates
//! on `&mut [Continent]` so foreign-based players are reachable and
//! both flows funnel through one set of post-match writes.
//!
//! ## Submodules
//!
//! * [`squad`]   — world-aware squad construction + emergency call-up
//! * [`stats`]   — caps/goals/reputation, Elo, schedule writes
//! * [`lookups`] — country reputation/elo/name lookups
//! * [`continental`] — orchestrator for continental qualifier matches
//! * [`tournament`]  — post-match processor for global tournaments
//!
//! ## Public API
//!
//! Re-exported below so call sites read
//! `continent::national::world::build_world_match_squad(..)` rather
//! than reaching into a specific submodule.

pub mod continental;
pub mod lookups;
pub mod squad;
pub mod stats;
pub mod tournament;

pub use continental::simulate_world_national_competitions;
pub use lookups::{world_country_elo, world_country_name, world_country_reputation};
pub use squad::{build_world_match_squad, emergency_callups_total};
pub use stats::{
    apply_world_elo, apply_world_international_stats, record_world_country_schedule,
};
pub use tournament::apply_global_tournament_result;

#[cfg(test)]
mod tests;
