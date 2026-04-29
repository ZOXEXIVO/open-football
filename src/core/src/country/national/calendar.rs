//! Date predicates for international-break and tournament windows.
//! These are the only public entry points the rest of the simulator
//! uses to ask "is today a break day?" / "are we mid-tournament?".

use super::types::{BREAK_WINDOWS, TOURNAMENT_WINDOW};
use super::NationalTeam;
use chrono::{Datelike, NaiveDate};

impl NationalTeam {
    pub fn is_break_start(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, start, _)| month == *m && day == *start)
    }

    pub fn is_break_end(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, _, end)| month == *m && day == *end)
    }

    pub fn is_in_break(date: NaiveDate) -> bool {
        let month = date.month();
        let day = date.day();
        BREAK_WINDOWS
            .iter()
            .any(|(m, start, end)| month == *m && day >= *start && day <= *end)
    }

    pub fn is_tournament_start(date: NaiveDate) -> bool {
        date.month() == TOURNAMENT_WINDOW.0 && date.day() == TOURNAMENT_WINDOW.1
    }

    pub fn is_tournament_end(date: NaiveDate) -> bool {
        date.month() == TOURNAMENT_WINDOW.2 && date.day() == TOURNAMENT_WINDOW.3
    }

    pub(super) fn is_in_tournament_period(date: NaiveDate) -> bool {
        let month = date.month();
        (month == TOURNAMENT_WINDOW.0 && date.day() >= TOURNAMENT_WINDOW.1)
            || (month == TOURNAMENT_WINDOW.2 && date.day() <= TOURNAMENT_WINDOW.3)
    }
}
