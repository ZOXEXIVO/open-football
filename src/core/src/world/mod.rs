//! The world: every continent, country, club and loose player the save
//! holds, plus the passes that keep that state coherent between ticks.
//!
//! [`SimulatorData`] is the aggregate root. The submodules each own one
//! slice of its behaviour and add their own `impl SimulatorData` block,
//! so callers still reach everything through the root — `data.sweep_…`,
//! `data.process_…` — while the implementations stay separated by
//! concern:
//!
//! * [`bootstrap`] — one-time and catch-up seeding (league tables,
//!   career histories, id sequences, passports).
//! * [`market`] — the transfer geography rebuilt from `country_info`.
//! * [`free_agents`] — the unattached pool: intake, retirement, and the
//!   parent-club share of a loanee's wages.
//! * [`national`] — world-level call-ups and their release.
//!
//! The daily tick that drives these lives one level up in
//! [`crate::simulator`]; nothing here knows about ordering.

pub(crate) mod bootstrap;
mod country_info;
mod data;
mod flow;
pub(crate) mod free_agents;
mod market;
mod national;

pub use country_info::CountryInfo;
pub use data::SimulatorData;
pub use flow::FreeAgentFlowCounters;
