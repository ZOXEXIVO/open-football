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

/// One team's window while every schedule is being read: only the four
/// nearest dates on each side are ever kept, rather than the season's
/// whole fixture list sorted and cut afterwards.
#[derive(Debug, Default)]
pub struct NearestFixtures {
    upcoming: Vec<NaiveDate>,
    recent: Vec<NaiveDate>,
}

impl NearestFixtures {
    const KEPT: usize = 4;

    pub fn offer_upcoming(&mut self, date: NaiveDate) {
        Self::keep(&mut self.upcoming, date, |new, kept| new < kept);
    }

    pub fn offer_recent(&mut self, date: NaiveDate) {
        Self::keep(&mut self.recent, date, |new, kept| new > kept);
    }

    /// Upcoming oldest first, recent newest first.
    pub fn into_window(self, today: NaiveDate) -> TeamFixtureWindow {
        TeamFixtureWindow {
            refreshed: Some(today),
            upcoming: self.upcoming,
            recent: self.recent,
        }
    }

    fn keep(
        dates: &mut Vec<NaiveDate>,
        date: NaiveDate,
        nearer: impl Fn(NaiveDate, NaiveDate) -> bool,
    ) {
        let at = dates
            .iter()
            .position(|&kept| nearer(date, kept))
            .unwrap_or(dates.len());
        if at < Self::KEPT {
            if dates.len() == Self::KEPT {
                dates.pop();
            }
            dates.insert(at, date);
        }
    }
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
    fn nearest_fixtures_keep_what_a_full_sort_and_cut_keeps() {
        let today = d(10, 10);
        let fixtures = [
            d(11, 2),
            d(10, 3),
            d(10, 14),
            d(9, 20),
            d(10, 14),
            d(12, 1),
            d(10, 9),
            d(10, 21),
            d(9, 27),
            d(10, 3),
            d(10, 11),
            d(10, 1),
        ];
        let mut nearest = NearestFixtures::default();
        for &f in &fixtures {
            if f > today {
                nearest.offer_upcoming(f);
            } else {
                nearest.offer_recent(f);
            }
        }
        let window = nearest.into_window(today);

        let mut upcoming: Vec<NaiveDate> =
            fixtures.iter().copied().filter(|f| *f > today).collect();
        upcoming.sort_unstable();
        upcoming.truncate(4);
        let mut recent: Vec<NaiveDate> = fixtures.iter().copied().filter(|f| *f <= today).collect();
        recent.sort_unstable_by(|a, b| b.cmp(a));
        recent.truncate(4);

        assert_eq!(window.refreshed, Some(today));
        assert_eq!(window.upcoming, upcoming);
        assert_eq!(window.recent, recent);
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
