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

    /// Number of fixtures (recent or upcoming) within `days` calendar
    /// days of `today`. Drives the double-match-week dampener.
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
