use crate::league::DayMonthPeriod;
use chrono::{Datelike, Duration, NaiveDate};

#[derive(Debug, Clone)]
pub struct Season {
    pub display: String,
    pub start_year: u16,
}

impl Season {
    pub fn new(start_year: u16) -> Self {
        let end_year = start_year + 1;
        Season {
            display: format!("{}/{}", start_year, end_year % 100),
            start_year,
        }
    }

    /// Determine which season a date falls in.
    /// Seasons run Aug–Jul: Aug 2033 → season 2033/34, Jun 2033 → season 2032/33.
    pub fn from_date(date: NaiveDate) -> Self {
        let start_year = if date.month() >= 8 {
            date.year() as u16
        } else {
            (date.year() - 1) as u16
        };
        Self::new(start_year)
    }

    /// Approximate start date of this season (Aug 1).
    pub fn start_date(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.start_year as i32, 8, 1).unwrap()
    }

    /// Approximate end date of this season (May 31 of next year).
    pub fn end_date(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.start_year as i32 + 1, 5, 31).unwrap()
    }

    /// The Monday of season week `n`. Week 0 is the first Monday on or
    /// after 29 August, which puts FIFA's September, October and November
    /// windows on weeks 0, 5 and 10.
    pub fn week(&self, n: u32) -> NaiveDate {
        let anchor = NaiveDate::from_ymd_opt(self.start_year as i32, 8, 29).unwrap();
        let to_monday = (7 - anchor.weekday().num_days_from_monday()) % 7;
        anchor + Duration::days((to_monday + 7 * n) as i64)
    }

    /// This season as a campaign of a league that runs August to July, for
    /// football no league calendar reaches.
    pub fn as_league_season(&self) -> LeagueSeason {
        LeagueSeason {
            opening_year: self.start_year as i32,
            crosses_new_year: true,
        }
    }
}

/// One campaign of a particular league, named by the year it opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeagueSeason {
    pub opening_year: i32,
    pub crosses_new_year: bool,
}

impl LeagueSeason {
    pub fn label(&self) -> String {
        if self.crosses_new_year {
            format!(
                "{}/{:02}",
                self.opening_year,
                (self.opening_year + 1).rem_euclid(100)
            )
        } else {
            self.opening_year.to_string()
        }
    }
}

/// A league's own season boundaries, read off its start and end windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeasonCalendar {
    opening: (u32, u32),
    crosses_new_year: bool,
    turnover_lead: i64,
}

impl SeasonCalendar {
    /// Seasons turn over halfway through the off-season rather than on
    /// opening day, so a fixture moved a day or two either side of the
    /// window — an eve-of-round youth fixture, a playoff after the close —
    /// still files under its own campaign.
    pub fn new(starting_half: &DayMonthPeriod, ending_half: &DayMonthPeriod) -> Self {
        let crosses_new_year = ending_half.to_month <= starting_half.from_month;

        // A leap reference year keeps a configured 29 February valid.
        let ordinal = |month: u8, day: u8| {
            NaiveDate::from_ymd_opt(2000, month as u32, day as u32)
                .unwrap()
                .ordinal() as i64
        };
        let opens = ordinal(starting_half.from_month, starting_half.from_day);
        let closes = ordinal(ending_half.to_month, ending_half.to_day);
        let off_season = if crosses_new_year {
            opens - closes
        } else {
            opens + 366 - closes
        };

        SeasonCalendar {
            opening: (
                starting_half.from_month as u32,
                starting_half.from_day as u32,
            ),
            crosses_new_year,
            turnover_lead: off_season.max(0) / 2,
        }
    }

    pub fn season_of(&self, date: NaiveDate) -> LeagueSeason {
        let shifted = date + Duration::days(self.turnover_lead);
        let year = if (shifted.month(), shifted.day()) >= self.opening {
            shifted.year()
        } else {
            shifted.year() - 1
        };
        self.season_opening_in(year)
    }

    pub fn season_opening_in(&self, year: i32) -> LeagueSeason {
        LeagueSeason {
            opening_year: year,
            crosses_new_year: self.crosses_new_year,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn calendar(opens: (u8, u8), closes: (u8, u8)) -> SeasonCalendar {
        let (open_day, open_month) = opens;
        let (close_day, close_month) = closes;
        SeasonCalendar::new(
            &DayMonthPeriod::new(open_day, open_month, 31, 12),
            &DayMonthPeriod::new(1, 1, close_day, close_month),
        )
    }

    #[test]
    fn season_weeks_reproduce_fifas_autumn_windows() {
        let cases = [
            (2023, [date(2023, 9, 4), date(2023, 10, 9), date(2023, 11, 13)]),
            (2024, [date(2024, 9, 2), date(2024, 10, 7), date(2024, 11, 11)]),
            (2025, [date(2025, 9, 1), date(2025, 10, 6), date(2025, 11, 10)]),
        ];
        for (year, expected) in cases {
            let season = Season::new(year);
            assert_eq!([season.week(0), season.week(5), season.week(10)], expected);
        }
    }

    #[test]
    fn a_season_week_starts_on_a_monday_from_29_august() {
        use chrono::Weekday;
        for year in 2026..=2060 {
            let week0 = Season::new(year).week(0);
            assert_eq!(week0.weekday(), Weekday::Mon);
            assert!(week0 >= date(year as i32, 8, 29) && week0 <= date(year as i32, 9, 4));
            assert_eq!(Season::new(year).week(3) - week0, Duration::days(21));
        }
    }

    #[test]
    fn a_campaign_across_the_new_year_is_labelled_by_both_years() {
        let season = LeagueSeason {
            opening_year: 2026,
            crosses_new_year: true,
        };
        assert_eq!(season.label(), "2026/27");
    }

    #[test]
    fn a_calendar_year_campaign_is_labelled_by_its_year() {
        let season = LeagueSeason {
            opening_year: 2026,
            crosses_new_year: false,
        };
        assert_eq!(season.label(), "2026");
    }

    #[test]
    fn the_second_year_is_zero_padded_across_a_century() {
        let season = LeagueSeason {
            opening_year: 2099,
            crosses_new_year: true,
        };
        assert_eq!(season.label(), "2099/00");
    }

    #[test]
    fn a_spring_date_belongs_to_the_campaign_that_opened_the_previous_summer() {
        let season = calendar((9, 8), (17, 5)).season_of(date(2027, 3, 14));
        assert_eq!(season.opening_year, 2026);
        assert_eq!(season.label(), "2026/27");
    }

    #[test]
    fn a_calendar_year_league_keeps_its_campaign_in_one_year() {
        let calendar = calendar((1, 2), (15, 12));
        assert_eq!(calendar.season_of(date(2026, 3, 7)).label(), "2026");
        assert_eq!(calendar.season_of(date(2026, 11, 20)).label(), "2026");
    }

    #[test]
    fn an_opener_moved_to_the_eve_of_the_window_stays_in_the_new_campaign() {
        let season = calendar((9, 8), (17, 5)).season_of(date(2026, 8, 8));
        assert_eq!(season.opening_year, 2026);
    }

    #[test]
    fn a_playoff_after_the_close_stays_in_the_campaign_it_decides() {
        let season = calendar((15, 7), (1, 6)).season_of(date(2027, 6, 10));
        assert_eq!(season.opening_year, 2026);
    }

    #[test]
    fn an_off_season_across_the_new_year_turns_over_in_december() {
        // Closes 1 October, reopens 1 March: the break's midpoint is
        // mid-December, so a date just before Christmas already counts
        // towards the campaign that opens in the spring.
        let calendar = calendar((1, 3), (1, 10));
        assert_eq!(calendar.season_of(date(2026, 11, 1)).opening_year, 2026);
        assert_eq!(calendar.season_of(date(2026, 12, 20)).opening_year, 2027);
    }

    #[test]
    fn the_tightest_break_still_turns_over_between_the_campaigns() {
        // 15 December to 1 January: a 17-day break, turning over on the 24th.
        let calendar = calendar((1, 1), (15, 12));
        assert_eq!(calendar.season_of(date(2026, 12, 15)).opening_year, 2026);
        assert_eq!(calendar.season_of(date(2026, 12, 20)).opening_year, 2026);
        assert_eq!(calendar.season_of(date(2026, 12, 30)).opening_year, 2027);
        assert_eq!(calendar.season_of(date(2027, 1, 1)).opening_year, 2027);
    }
}
