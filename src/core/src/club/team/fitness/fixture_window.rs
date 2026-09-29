use chrono::Duration;
use chrono::NaiveDate;

/// Cached competitive-fixture window for this team. Populated by the
/// league/country pipeline before `Team::simulate` runs so training
/// can read real calendar distance to the next match. Friendlies are
/// excluded — they do not earn the same MD-1 / MD-2 protection.
#[derive(Debug, Clone, Default)]
pub struct TeamFixtureWindow {
    /// Date this window was last refreshed. Lets training tell the
    /// difference between "no fixtures because there are none" and
    /// "no fixtures because the cache was never written".
    pub refreshed: Option<NaiveDate>,
    /// Up to four upcoming competitive match dates, oldest first.
    pub upcoming: Vec<NaiveDate>,
    /// Up to four most recent competitive match dates, newest first.
    pub recent: Vec<NaiveDate>,
}

impl TeamFixtureWindow {
    pub fn next_after(&self, today: NaiveDate) -> Option<NaiveDate> {
        self.upcoming.iter().copied().find(|d| *d >= today)
    }

    pub fn previous_before(&self, today: NaiveDate) -> Option<NaiveDate> {
        self.recent.iter().copied().find(|d| *d <= today)
    }

    /// Competitive fixtures in the seven days centred on `today`: two or
    /// more is a double-match week, which training plans around.
    pub fn fixtures_this_week(&self, today: NaiveDate) -> u8 {
        self.fixtures_within(today, 3)
    }

    /// Number of fixtures (recent or upcoming) within `days` calendar
    /// days of `today`.
    pub fn fixtures_within(&self, today: NaiveDate, days: i64) -> u8 {
        let half = Duration::days(days);
        let lo = today - half;
        let hi = today + half;
        let r = self
            .recent
            .iter()
            .filter(|d| **d >= lo && **d <= today)
            .count();
        let u = self
            .upcoming
            .iter()
            .filter(|d| **d > today && **d <= hi)
            .count();
        (r + u).min(u8::MAX as usize) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).unwrap()
    }

    /// The window as the country pipeline writes it on `today`: fixtures
    /// already played are recent (newest first), the rest upcoming.
    fn window(today: NaiveDate, fixtures: &[NaiveDate]) -> TeamFixtureWindow {
        let mut recent: Vec<NaiveDate> = fixtures.iter().copied().filter(|f| *f <= today).collect();
        recent.sort_unstable_by(|a, b| b.cmp(a));
        TeamFixtureWindow {
            refreshed: Some(today),
            upcoming: fixtures.iter().copied().filter(|f| *f > today).collect(),
            recent,
        }
    }

    #[test]
    fn a_weekly_schedule_is_never_a_double_match_week() {
        let saturdays = [d(10, 3), d(10, 10), d(10, 17), d(10, 24)];
        for day in 4..=20 {
            let today = d(10, day);
            assert!(
                window(today, &saturdays).fixtures_this_week(today) < 2,
                "one match a week read as congested on {today}"
            );
        }
    }

    #[test]
    fn a_midweek_fixture_makes_a_double_match_week() {
        let fixtures = [d(10, 3), d(10, 7), d(10, 10)];
        for today in [d(10, 5), d(10, 7), d(10, 8)] {
            assert!(
                window(today, &fixtures).fixtures_this_week(today) >= 2,
                "Saturday-Wednesday-Saturday not read as congested on {today}"
            );
        }
    }
}
