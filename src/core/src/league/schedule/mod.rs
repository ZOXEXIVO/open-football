pub mod cup;
pub mod result;
pub mod round;
pub mod schedule;

use crate::league::{LeagueSettings, Season};
pub use result::*;
pub use schedule::*;

pub trait ScheduleGenerator {
    fn generate(
        &self,
        league_id: u32,
        league_slug: &str,
        season: Season,
        teams: &[u32],
        league_settings: &LeagueSettings,
    ) -> Result<Vec<ScheduleTour>, ScheduleError>;
}
