use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

/// Which football a competition carries. Development (youth) football is
/// played the day before the senior round, so a club's youth side never
/// shares a matchday with its senior sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompetitionLevel {
    Senior,
    Development,
}

impl CompetitionLevel {
    pub fn round_weekday(self) -> Weekday {
        match self {
            CompetitionLevel::Senior => Weekday::Sat,
            CompetitionLevel::Development => Weekday::Fri,
        }
    }

    /// The day of a midweek round, when a league has more rounds than
    /// weekends: the senior Tuesday, and the day after it for development.
    pub fn midweek_weekday(self) -> Weekday {
        match self {
            CompetitionLevel::Senior => Weekday::Tue,
            CompetitionLevel::Development => Weekday::Wed,
        }
    }
}

/// When a domestic fixture kicks off. The simulation clock only ever reads
/// midnight, so the kickoff is what the calendar shows; `slot` staggers a
/// day's fixtures across its programme.
pub struct KickoffClock;

impl KickoffClock {
    const SENIOR_WEEKEND: [(u32, u32); 4] = [(13, 0), (15, 0), (17, 30), (20, 0)];
    const SENIOR_WEEKDAY: [(u32, u32); 2] = [(19, 0), (20, 0)];
    const DEVELOPMENT: [(u32, u32); 2] = [(11, 0), (13, 0)];

    pub fn at(day: NaiveDate, level: CompetitionLevel, slot: usize) -> NaiveDateTime {
        let slots: &[(u32, u32)] = match level {
            CompetitionLevel::Development => &Self::DEVELOPMENT,
            CompetitionLevel::Senior if matches!(day.weekday(), Weekday::Sat | Weekday::Sun) => {
                &Self::SENIOR_WEEKEND
            }
            CompetitionLevel::Senior => &Self::SENIOR_WEEKDAY,
        };
        let (hour, minute) = slots[slot % slots.len()];
        day.and_time(NaiveTime::from_hms_opt(hour, minute, 0).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn hours(day: NaiveDate, level: CompetitionLevel) -> Vec<f32> {
        (0..8)
            .map(|slot| {
                let t = KickoffClock::at(day, level, slot);
                assert_eq!(t.date(), day, "a kickoff never leaves its day");
                t.hour() as f32 + t.minute() as f32 / 60.0
            })
            .collect()
    }

    #[test]
    fn senior_weekend_kicks_off_in_the_afternoon_or_evening() {
        for day in [d(2026, 8, 8), d(2026, 8, 9)] {
            for h in hours(day, CompetitionLevel::Senior) {
                assert!((12.0..=21.0).contains(&h), "{day}: {h}");
            }
        }
    }

    #[test]
    fn senior_weekday_kicks_off_in_the_evening() {
        for day in [d(2026, 8, 10), d(2026, 8, 12), d(2026, 8, 14)] {
            for h in hours(day, CompetitionLevel::Senior) {
                assert!((18.0..=21.0).contains(&h), "{day}: {h}");
            }
        }
    }

    #[test]
    fn a_senior_midweek_round_is_a_tuesday_evening() {
        let level = CompetitionLevel::Senior;
        assert_eq!(level.midweek_weekday(), Weekday::Tue);
        for h in hours(d(2026, 8, 11), level) {
            assert!((18.0..=21.0).contains(&h), "{h}");
        }
    }

    #[test]
    fn a_development_midweek_round_follows_the_senior_one() {
        assert_eq!(
            CompetitionLevel::Development.midweek_weekday(),
            CompetitionLevel::Senior.midweek_weekday().succ()
        );
    }

    #[test]
    fn development_kicks_off_late_morning_or_midday() {
        for day in [d(2026, 8, 7), d(2026, 8, 8), d(2026, 8, 12)] {
            for h in hours(day, CompetitionLevel::Development) {
                assert!((10.0..=14.0).contains(&h), "{day}: {h}");
            }
        }
    }

    #[test]
    fn slots_wrap_deterministically() {
        let day = d(2026, 8, 8);
        let level = CompetitionLevel::Senior;
        assert_eq!(
            KickoffClock::at(day, level, 1),
            KickoffClock::at(day, level, 5)
        );
        assert_ne!(
            KickoffClock::at(day, level, 0),
            KickoffClock::at(day, level, 1)
        );
    }
}
