//! Date predicates for international windows and the tournament window.
//! These are the only public entry points the rest of the simulator
//! uses to ask "is today a window day?" / "are we mid-tournament?".

use super::NationalTeam;
use super::types::TOURNAMENT_WINDOW;
use crate::league::Season;
use chrono::{Datelike, Duration, NaiveDate};

/// FIFA's international calendar: four windows a season on the season
/// week grid, each running Monday to the Tuesday eight days later so it
/// holds exactly one weekend.
pub struct InternationalCalendar;

impl InternationalCalendar {
    /// Each window by the month that names it and the season week it
    /// opens: September, October, November and March.
    const WEEKS: [(u32, u32); 4] = [(9, 0), (10, 5), (11, 10), (3, 29)];

    pub fn windows(season: &Season) -> impl Iterator<Item = InternationalWindow> + '_ {
        Self::WEEKS
            .iter()
            .map(|(_, week)| InternationalWindow::opening(season.week(*week)))
    }

    pub fn window_on(date: NaiveDate) -> Option<InternationalWindow> {
        Self::windows(&Season::from_date(date)).find(|window| window.contains(date))
    }

    pub fn names_window(month: u32) -> bool {
        Self::WEEKS.iter().any(|(named, _)| *named == month)
    }

    /// The window a configured date names by its month, in `season`.
    pub fn window_for(season: &Season, month: u32) -> Option<InternationalWindow> {
        Self::WEEKS
            .iter()
            .find(|(named, _)| *named == month)
            .map(|(_, week)| InternationalWindow::opening(season.week(*week)))
    }

    pub fn opens_on(date: NaiveDate) -> bool {
        Self::window_on(date).is_some_and(|window| window.opens == date)
    }

    pub fn closes_on(date: NaiveDate) -> bool {
        Self::window_on(date).is_some_and(|window| window.closes == date)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InternationalWindow {
    pub opens: NaiveDate,
    pub closes: NaiveDate,
}

impl InternationalWindow {
    /// National-team matchdays by days after the opening Monday: the
    /// Thursday, the Sunday and the closing Tuesday.
    const MATCHDAY_OFFSETS: [i64; 3] = [3, 6, 8];
    pub const MATCHDAYS: usize = Self::MATCHDAY_OFFSETS.len();

    fn opening(opens: NaiveDate) -> Self {
        InternationalWindow {
            opens,
            closes: opens + Duration::days(8),
        }
    }

    pub fn contains(&self, date: NaiveDate) -> bool {
        date >= self.opens && date <= self.closes
    }

    pub fn matchday(&self, slot: usize) -> NaiveDate {
        self.opens + Duration::days(Self::MATCHDAY_OFFSETS[slot])
    }
}

impl NationalTeam {
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Weekday;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn every_season_has_four_monday_to_tuesday_windows_in_order() {
        for year in 2026..=2060 {
            let windows: Vec<InternationalWindow> =
                InternationalCalendar::windows(&Season::new(year)).collect();
            assert_eq!(windows.len(), 4, "season {year}");
            for pair in windows.windows(2) {
                assert!(pair[0].closes < pair[1].opens, "season {year} out of order");
            }
            for window in &windows {
                assert_eq!(window.opens.weekday(), Weekday::Mon, "{window:?}");
                assert_eq!(window.closes.weekday(), Weekday::Tue, "{window:?}");
                assert_eq!(window.closes - window.opens, Duration::days(8));
            }
        }
    }

    #[test]
    fn every_window_holds_exactly_one_weekend() {
        for year in 2026..=2060 {
            for window in InternationalCalendar::windows(&Season::new(year)) {
                for weekday in [Weekday::Fri, Weekday::Sat, Weekday::Sun] {
                    let count = (0..=8)
                        .map(|k| window.opens + Duration::days(k))
                        .filter(|day| day.weekday() == weekday)
                        .count();
                    assert_eq!(count, 1, "{weekday:?} in {window:?}");
                }
            }
        }
    }

    #[test]
    fn the_2027_28_windows() {
        let windows: Vec<(NaiveDate, NaiveDate)> =
            InternationalCalendar::windows(&Season::new(2027))
                .map(|w| (w.opens, w.closes))
                .collect();
        assert_eq!(
            windows,
            vec![
                (d(2027, 8, 30), d(2027, 9, 7)),
                (d(2027, 10, 4), d(2027, 10, 12)),
                (d(2027, 11, 8), d(2027, 11, 16)),
                (d(2028, 3, 20), d(2028, 3, 28)),
            ]
        );
    }

    #[test]
    fn a_date_finds_its_window_across_the_season_boundary() {
        // 30 Aug 2027 opens the September window of the 2027 season; late
        // March 2028 still belongs to that season's March window.
        assert_eq!(
            InternationalCalendar::window_on(d(2027, 8, 30)).map(|w| w.closes),
            Some(d(2027, 9, 7))
        );
        assert_eq!(
            InternationalCalendar::window_on(d(2028, 3, 28)).map(|w| w.opens),
            Some(d(2028, 3, 20))
        );
        assert_eq!(InternationalCalendar::window_on(d(2027, 9, 8)), None);
        assert_eq!(InternationalCalendar::window_on(d(2027, 8, 29)), None);
    }

    #[test]
    fn a_window_opens_and_closes_on_one_day_each() {
        let season = Season::new(2027);
        for window in InternationalCalendar::windows(&season) {
            for k in 0..=8 {
                let day = window.opens + Duration::days(k);
                assert_eq!(InternationalCalendar::opens_on(day), k == 0, "{day}");
                assert_eq!(InternationalCalendar::closes_on(day), k == 8, "{day}");
            }
        }
    }

    #[test]
    fn matchdays_are_thursday_sunday_and_the_closing_tuesday() {
        let window = InternationalCalendar::window_for(&Season::new(2027), 10).unwrap();
        assert_eq!(window.matchday(0), d(2027, 10, 7));
        assert_eq!(window.matchday(1), d(2027, 10, 10));
        assert_eq!(window.matchday(2), window.closes);
    }

    #[test]
    fn only_the_four_window_months_name_a_window() {
        let season = Season::new(2027);
        for month in 1..=12 {
            assert_eq!(
                InternationalCalendar::window_for(&season, month).is_some(),
                [3, 9, 10, 11].contains(&month),
                "month {month}"
            );
        }
    }
}
