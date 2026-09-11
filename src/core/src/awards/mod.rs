//! World-level award ticks.
//!
//! Each tick reads the world, decides who the week / month / season
//! belonged to, and writes the result onto the league or continent that
//! keeps the shelf. The award *types* live with their owners
//! ([`crate::league::awards`]); what lives here is the selection.
//!
//! [`MondayAwardCache`] is the shared read: all four Monday tickers need
//! the same per-league weekly aggregates, so they are built once and
//! passed around rather than recomputed per tick.

pub(crate) mod cache;
mod monthly;
mod season;
mod team_of_week;
mod team_of_week_young;
mod team_of_year;
mod weekly;
mod weekly_young;
mod world_poy;

pub(crate) use cache::MondayAwardCache;
pub(crate) use monthly::MonthlyAwardsTick;
pub(crate) use season::SeasonAwardsTick;
pub(crate) use team_of_week::TeamOfTheWeekTick;
pub(crate) use team_of_week_young::YoungTeamOfTheWeekTick;
pub(crate) use team_of_year::TeamOfTheYearTick;
pub(crate) use weekly::WeeklyAwardsTick;
pub(crate) use weekly_young::YoungWeeklyAwardsTick;
pub(crate) use world_poy::WorldPlayerOfYearTick;
