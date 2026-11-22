use crate::club::player::events::discipline::CompetitionFamily;
use chrono::Duration;
use chrono::NaiveDate;

/// A competitive fixture still to be played, and what its competition says
/// it is worth to this side as things stand.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UpcomingFixture {
    pub date: NaiveDate,
    pub importance: f32,
}

/// One owner's nearest fixtures either side of today.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FixtureRun {
    /// Up to four upcoming fixtures, oldest first.
    pub upcoming: Vec<UpcomingFixture>,
    /// Up to four most recent fixture dates, newest first.
    pub recent: Vec<NaiveDate>,
}

impl FixtureRun {
    /// A run of bare dates, each at `importance`.
    pub fn dated(upcoming: &[NaiveDate], recent: &[NaiveDate], importance: f32) -> Self {
        FixtureRun {
            upcoming: upcoming
                .iter()
                .map(|&date| UpcomingFixture { date, importance })
                .collect(),
            recent: recent.to_vec(),
        }
    }
}

/// Cached competitive-fixture window for this team, where its competitions
/// meet. Friendlies are excluded — they do not earn the same MD-1 / MD-2
/// protection.
#[derive(Debug, Clone, Default)]
pub struct TeamFixtureWindow {
    /// Date this window was last refreshed. Lets training tell the
    /// difference between "no fixtures because there are none" and
    /// "no fixtures because the cache was never written".
    pub refreshed: Option<NaiveDate>,
    /// League, play-off and domestic cup fixtures, written by the country
    /// once the day's matches are played.
    pub domestic: FixtureRun,
    /// Continental club fixtures, written by the continent.
    pub continental: FixtureRun,
    /// Development-league fixtures, which nobody plans a week around but
    /// which still put a side on a pitch.
    pub development: FixtureRun,
}

/// One team's window while every schedule is being read: only the four
/// nearest fixtures on each side are ever kept, rather than the season's
/// whole fixture list sorted and cut afterwards.
#[derive(Debug, Default)]
pub struct NearestFixtures {
    upcoming: Vec<UpcomingFixture>,
    recent: Vec<NaiveDate>,
}

impl NearestFixtures {
    const KEPT: usize = 4;

    pub fn offer_upcoming(&mut self, date: NaiveDate, importance: f32) {
        let at = self
            .upcoming
            .iter()
            .position(|kept| date < kept.date)
            .unwrap_or(self.upcoming.len());
        if at < Self::KEPT {
            if self.upcoming.len() == Self::KEPT {
                self.upcoming.pop();
            }
            self.upcoming
                .insert(at, UpcomingFixture { date, importance });
        }
    }

    pub fn offer_recent(&mut self, date: NaiveDate) {
        let at = self
            .recent
            .iter()
            .position(|&kept| date > kept)
            .unwrap_or(self.recent.len());
        if at < Self::KEPT {
            if self.recent.len() == Self::KEPT {
                self.recent.pop();
            }
            self.recent.insert(at, date);
        }
    }

    /// Upcoming oldest first, recent newest first.
    pub fn into_run(self) -> FixtureRun {
        FixtureRun {
            upcoming: self.upcoming,
            recent: self.recent,
        }
    }
}

/// A club's week around one fixture, across every competition it is in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClubWeek {
    /// Days since the last competitive fixture.
    pub since_last: Option<i64>,
    /// Days until the next one.
    pub to_next: Option<i64>,
    /// What the next one is worth to the club.
    pub next_importance: f32,
}

impl ClubWeek {
    /// A fixture this near is the same week; one this far is the next.
    const NEAR_DAYS: f32 = 2.0;
    const FAR_DAYS: f32 = 7.0;

    pub fn free() -> Self {
        ClubWeek {
            since_last: None,
            to_next: None,
            next_importance: 0.0,
        }
    }

    /// Nothing competitive within a week either side.
    pub fn is_free(&self) -> bool {
        let clear = |days: Option<i64>| days.is_none_or(|d| d as f32 >= Self::FAR_DAYS);
        clear(self.since_last) && clear(self.to_next)
    }

    /// How much the next fixture bears on this one: whole when it is two
    /// days off, nothing from a week.
    pub fn nearness_of_next(&self) -> f32 {
        match self.to_next {
            Some(days) => ((Self::FAR_DAYS - days as f32) / (Self::FAR_DAYS - Self::NEAR_DAYS))
                .clamp(0.0, 1.0),
            None => 0.0,
        }
    }
}

impl Default for ClubWeek {
    fn default() -> Self {
        Self::free()
    }
}

impl TeamFixtureWindow {
    fn upcoming(&self) -> impl Iterator<Item = &UpcomingFixture> {
        self.domestic
            .upcoming
            .iter()
            .chain(self.continental.upcoming.iter())
    }

    fn recent(&self) -> impl Iterator<Item = NaiveDate> + '_ {
        self.domestic
            .recent
            .iter()
            .chain(self.continental.recent.iter())
            .copied()
    }

    /// A competitive fixture has been moved. The window is rewritten from
    /// the schedules each evening; until then it has to stay true for the
    /// day the fixture now falls on.
    pub fn on_fixture_moved(&mut self, from: NaiveDate, to: NaiveDate) {
        let upcoming = &mut self.domestic.upcoming;
        let importance = match upcoming.iter().position(|f| f.date == from) {
            Some(at) => upcoming.remove(at).importance,
            None => upcoming.first().map_or(0.0, |f| f.importance),
        };
        let at = upcoming
            .iter()
            .position(|f| to < f.date)
            .unwrap_or(upcoming.len());
        upcoming.insert(
            at,
            UpcomingFixture {
                date: to,
                importance,
            },
        );
    }

    /// The side has a fixture of any kind on `date`.
    pub fn plays_on(&self, date: NaiveDate) -> bool {
        self.competes_on(date).is_some() || self.development.upcoming.iter().any(|f| f.date == date)
    }

    /// The family of the competitive fixture the side has on `date`.
    pub fn competes_on(&self, date: NaiveDate) -> Option<CompetitionFamily> {
        let on = |run: &FixtureRun| run.upcoming.iter().any(|f| f.date == date);
        if on(&self.continental) {
            Some(CompetitionFamily::Continental)
        } else if on(&self.domestic) {
            Some(CompetitionFamily::Domestic)
        } else {
            None
        }
    }

    pub fn next_after(&self, today: NaiveDate) -> Option<NaiveDate> {
        self.upcoming()
            .map(|f| f.date)
            .filter(|d| *d >= today)
            .min()
    }

    pub fn previous_before(&self, today: NaiveDate) -> Option<NaiveDate> {
        self.recent().filter(|d| *d <= today).max()
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
        let r = self.recent().filter(|d| *d >= lo && *d <= today).count();
        let u = self
            .upcoming()
            .filter(|f| f.date > today && f.date <= hi)
            .count();
        (r + u).min(u8::MAX as usize) as u8
    }

    /// The family of competitions the next fixture after `today` belongs
    /// to. A side with nothing in sight awaits a domestic one.
    pub fn next_family(&self, today: NaiveDate) -> CompetitionFamily {
        let first = |run: &FixtureRun| {
            run.upcoming
                .iter()
                .map(|f| f.date)
                .filter(|d| *d > today)
                .min()
        };
        match (first(&self.domestic), first(&self.continental)) {
            (Some(domestic), Some(continental)) if continental < domestic => {
                CompetitionFamily::Continental
            }
            (None, Some(_)) => CompetitionFamily::Continental,
            _ => CompetitionFamily::Domestic,
        }
    }

    /// The week around a fixture played `today`: the last fixture before
    /// it and the next after it, in any competition.
    pub fn week(&self, today: NaiveDate) -> ClubWeek {
        let next = self
            .upcoming()
            .filter(|f| f.date > today)
            .min_by_key(|f| f.date);
        ClubWeek {
            since_last: self
                .recent()
                .filter(|d| *d < today)
                .max()
                .map(|d| (today - d).num_days()),
            to_next: next.map(|f| (f.date - today).num_days()),
            next_importance: next.map(|f| f.importance).unwrap_or(0.0),
        }
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
        let upcoming: Vec<NaiveDate> = fixtures.iter().copied().filter(|f| *f > today).collect();
        TeamFixtureWindow {
            refreshed: Some(today),
            domestic: FixtureRun::dated(&upcoming, &recent, 0.6),
            continental: FixtureRun::default(),
            development: FixtureRun::default(),
        }
    }

    #[test]
    fn a_moved_fixture_is_read_on_its_new_day_at_once() {
        let mut moved = window(d(10, 5), &[d(10, 10), d(10, 14), d(10, 17)]);
        moved.on_fixture_moved(d(10, 14), d(10, 6));
        assert_eq!(
            moved.competes_on(d(10, 6)),
            Some(CompetitionFamily::Domestic)
        );
        assert_eq!(moved.competes_on(d(10, 14)), None);
        assert_eq!(
            moved
                .domestic
                .upcoming
                .iter()
                .map(|f| f.date)
                .collect::<Vec<_>>(),
            vec![d(10, 6), d(10, 10), d(10, 17)]
        );
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
                nearest.offer_upcoming(f, 0.6);
            } else {
                nearest.offer_recent(f);
            }
        }
        let run = nearest.into_run();

        let mut upcoming: Vec<NaiveDate> =
            fixtures.iter().copied().filter(|f| *f > today).collect();
        upcoming.sort_unstable();
        upcoming.truncate(4);
        let mut recent: Vec<NaiveDate> = fixtures.iter().copied().filter(|f| *f <= today).collect();
        recent.sort_unstable_by(|a, b| b.cmp(a));
        recent.truncate(4);

        assert_eq!(
            run.upcoming.iter().map(|f| f.date).collect::<Vec<_>>(),
            upcoming
        );
        assert_eq!(run.recent, recent);
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

    /// A club with a league match on Saturday the 10th and a continental
    /// tie on Tuesday the 13th, as its window stands on each morning.
    fn european_week(refreshed: NaiveDate) -> TeamFixtureWindow {
        let league = [d(10, 3), d(10, 10), d(10, 17)];
        let mut window = window(refreshed, &league);
        window.continental = FixtureRun {
            upcoming: vec![UpcomingFixture {
                date: d(10, 13),
                importance: 0.9,
            }],
            recent: vec![d(9, 29)],
        };
        window
    }

    #[test]
    fn a_league_match_is_told_of_the_european_tie_three_days_on() {
        let week = european_week(d(10, 9)).week(d(10, 10));
        assert_eq!(week.to_next, Some(3));
        assert_eq!(week.next_importance, 0.9);
        assert_eq!(week.since_last, Some(7));
        assert!(!week.is_free());
        assert!(week.nearness_of_next() > 0.7);
    }

    #[test]
    fn a_european_tie_is_told_of_the_league_match_that_follows() {
        // Built on the 13th from the window as the 12th left it: the tie
        // itself is still listed as upcoming there.
        let week = european_week(d(10, 12)).week(d(10, 13));
        assert_eq!(week.to_next, Some(4), "Saturday's league match");
        assert_eq!(week.next_importance, 0.6);
        assert_eq!(week.since_last, Some(3));
    }

    #[test]
    fn fixtures_a_week_either_side_leave_the_week_free() {
        let saturdays = [d(10, 3), d(10, 10), d(10, 17)];
        let week = window(d(10, 9), &saturdays).week(d(10, 10));
        assert_eq!((week.since_last, week.to_next), (Some(7), Some(7)));
        assert!(week.is_free());
        assert_eq!(week.nearness_of_next(), 0.0);
        assert!(ClubWeek::free().is_free());
    }

    #[test]
    fn the_next_fixture_names_its_family() {
        assert_eq!(
            european_week(d(10, 10)).next_family(d(10, 10)),
            CompetitionFamily::Continental,
            "Tuesday's tie comes before next Saturday"
        );
        assert_eq!(
            european_week(d(10, 9)).next_family(d(10, 9)),
            CompetitionFamily::Domestic,
            "Saturday's league match comes first"
        );
        assert_eq!(
            TeamFixtureWindow::default().next_family(d(10, 9)),
            CompetitionFamily::Domestic
        );
    }

    #[test]
    fn training_counts_a_continental_night() {
        let window = european_week(d(10, 11));
        assert!(window.fixtures_this_week(d(10, 11)) >= 2);
        assert_eq!(window.next_after(d(10, 11)), Some(d(10, 13)));
    }
}
