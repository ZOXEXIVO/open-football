pub mod cup;
pub mod kickoff;
pub mod rest;
pub mod result;
pub mod round;
pub mod round_calendar;
pub mod schedule;

use crate::league::{LeagueSettings, Season};
pub use kickoff::*;
pub use rest::*;
pub use result::*;
pub use round_calendar::*;
pub use schedule::*;

pub trait ScheduleGenerator {
    fn generate(
        &self,
        league_id: u32,
        league_slug: &str,
        season: Season,
        teams: &[u32],
        league_settings: &LeagueSettings,
        level: CompetitionLevel,
    ) -> Result<Vec<ScheduleTour>, ScheduleError>;
}
