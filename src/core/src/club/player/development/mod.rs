//! A player's development, owned in one place. Training, matches, the academy
//! and mentoring report work; one rule turns it into attribute change:
//! progress along his blueprint, at his plasticity, as much as he takes in,
//! slowed as an attribute nears its ceiling and held inside his potential.
//! Ceilings are his potential shared out by his blueprint, at the share his
//! age holds; sessions and maturation fill them to what a player without
//! football holds, and only matches, banked per family for his career by how
//! well their standard fits him, open the rest. The weekly tick matures a
//! growing body, takes what an ageing one no longer sustains and the edge an
//! idle one loses. The ledger records every change by source for the census
//! and for the narrow read the mind takes of him.
//!
//! - [`blueprint`]: the profile a player develops toward, drawn once.
//! - [`ceilings`]: per-attribute ceilings at an age, with and without football.
//! - [`work`]: `SessionWork` and the growth rule.
//! - [`session`], [`football`], [`mentoring`]: what each kind of work does.
//! - [`tick`], [`layoff`]: time without work.
//! - [`timing`]: how early or late his body matures.
//! - [`ledger`]: change by source, and `DevelopmentRead`.

mod blueprint;
mod ceilings;
mod football;
mod layoff;
mod ledger;
mod mentoring;
mod session;
mod skills_array;
mod state;
mod tick;
mod timing;
mod work;

pub use blueprint::SkillBlueprint;
pub use ceilings::PositionalSkillCeilings;
pub use football::{BankedFootball, FootballFit, MatchFootball, SquadRole};
pub use layoff::Layoff;
pub use ledger::{DevelopmentRead, GrowthLedger, GrowthSource, SeasonGrowth};
pub use mentoring::Mentoring;
pub use skills_array::{SkillCategory, SkillKey};
pub use state::PlayerDevelopment;
pub use timing::MaturationTiming;
pub use work::SessionWork;

#[cfg(test)]
mod career_tests;
#[cfg(test)]
mod layering;
#[cfg(test)]
mod tests;
