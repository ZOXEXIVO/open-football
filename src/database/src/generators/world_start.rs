use chrono::{Datelike, Local, NaiveDate};
use core::league::Season;
use std::sync::OnceLock;

/// When a freshly generated world begins: 1 August of the current
/// real-world year — the top of the season the compiled database describes.
///
/// One definition, two readers. [`crate::DatabaseGenerator`] stamps it as
/// the simulator's opening date, and [`crate::generators::ClubJoinAnchor`]
/// measures each scraped career history against it to tell a summer signing
/// from a long-serving squad member. Were the two ever to disagree, every
/// player who moved in the window just gone would be dated a season out and
/// start the save looking settled.
///
/// The clock is read once per process. Hydration runs the join anchor on
/// every record across all worker threads, and `Local::now()` re-resolves
/// the timezone on each call.
pub struct WorldStart;

impl WorldStart {
    /// Opening date of the simulation.
    pub fn date() -> NaiveDate {
        static DATE: OnceLock<NaiveDate> = OnceLock::new();
        *DATE.get_or_init(|| {
            NaiveDate::from_ymd_opt(Local::now().year(), 8, 1)
                .expect("1 August exists in every year")
        })
    }

    /// Start year of the season the world opens in — 2026 for a world that
    /// begins on 2026-08-01 and plays the 2026/27 campaign.
    pub fn season_start_year() -> u16 {
        static YEAR: OnceLock<u16> = OnceLock::new();
        *YEAR.get_or_init(|| Season::from_date(Self::date()).start_year)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_world_opens_at_the_top_of_the_season_it_reports() {
        let date = WorldStart::date();
        assert_eq!((date.month(), date.day()), (8, 1));
        assert_eq!(WorldStart::season_start_year(), date.year() as u16);
        assert_eq!(
            Season::new(WorldStart::season_start_year()).start_date(),
            date,
            "the join anchor dates a fresh signing to the season start — it \
             has to land on the day the save actually opens"
        );
    }
}
