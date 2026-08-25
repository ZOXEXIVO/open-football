//! Team-level dynamics: in-match tactical state, chemistry between
//! players, coach influence, and zonal team shape.
//!
//! Grouped by what each part is:
//!
//! | Group | Concern |
//! |---|---|
//! | [`plans`] | The per-tick team plans: attack, defence, shape — who is doing what right now |
//! | [`tactical`] | [`TeamTacticalState`](tactical::TeamTacticalState) and how the game is read: phase, zones, signals |
//! | [`coach`] | The touchline: the instruction ladder, the metrics it watches, the substitution need |
//! | [`squad`] | Kickoff-latched properties of the two squads: pair chemistry, and the standard of football |
//!
//! [`zones`] stays at the root on purpose. It is the pitch-zone taxonomy
//! ([`MatchZone`](zones::MatchZone), [`LateralLane`](zones::LateralLane))
//! and the rating coefficients keyed on it — nothing in the four groups
//! above reads it; the statistics, event and rating layers do.
//!
//! Each group re-exports below under the module name it had before the
//! grouping, so every `teamplay::<module>::Item` path in the engine keeps
//! resolving.

pub mod coach;
pub mod plans;
pub mod squad;
pub mod tactical;
pub mod zones;

pub use plans::{attack, defence, shape, wide};
pub use squad::{chemistry, standard};
