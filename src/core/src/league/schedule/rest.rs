use crate::InternationalCalendar;
use chrono::{Duration, NaiveDate};

/// How far a domestic fixture moves to give both clubs a rest between their
/// matches — the rearrangement a league makes when a club is in Europe.
pub struct FixtureRest;

impl FixtureRest {
    /// Two clear days between matches: a Thursday tie is followed at the
    /// earliest by a Sunday game.
    pub const MIN_REST_DAYS: i64 = 3;
    /// Far enough to reach the next free midweek when a cup tie collides
    /// with a continental night. Counted in days outside international
    /// windows, so a window in the way doesn't shrink the reach.
    const SEARCH_DAYS: usize = 10;

    /// The nearest day, never before `today` and never inside an
    /// international window, on which both sides are rested from every one
    /// of their other fixtures; the later of two equally near days, since
    /// the clash is usually a midweek tie just before. With no rested day in
    /// reach, the day with the largest smaller gap.
    pub fn best_day(
        nominal: NaiveDate,
        today: NaiveDate,
        home_busy: &[NaiveDate],
        away_busy: &[NaiveDate],
    ) -> NaiveDate {
        let gap = |day: NaiveDate| {
            home_busy
                .iter()
                .chain(away_busy)
                .map(|busy| (day - *busy).num_days().abs())
                .min()
                .unwrap_or(i64::MAX)
        };

        let open = |day: &NaiveDate| InternationalCalendar::window_on(*day).is_none();
        let reach = |step: i64| {
            (1..)
                .map(move |k| nominal + Duration::days(step * k))
                .take_while(move |day| *day >= today)
                .filter(open)
                .take(Self::SEARCH_DAYS)
        };
        let mut candidates: Vec<NaiveDate> = reach(1).chain(reach(-1)).collect();
        if nominal >= today && open(&nominal) {
            candidates.push(nominal);
        }
        candidates.sort_by_key(|day| ((*day - nominal).num_days().abs(), *day < nominal));

        if let Some(day) = candidates
            .iter()
            .find(|day| gap(**day) >= Self::MIN_REST_DAYS)
        {
            return *day;
        }

        candidates
            .iter()
            .enumerate()
            .max_by_key(|(order, day)| (gap(**day), std::cmp::Reverse(*order)))
            .map(|(_, day)| *day)
            .unwrap_or(nominal)
    }

    pub fn is_rested(day: NaiveDate, busy: &[NaiveDate]) -> bool {
        busy.iter()
            .all(|b| (day - *b).num_days().abs() >= Self::MIN_REST_DAYS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap()
    }

    // September 2026: Saturdays 12 / 19 / 26, 3 October; the weeks' Tuesdays
    // 15 / 22 / 29, Wednesdays 16 / 23 / 30, Thursdays 17 / 24.

    #[test]
    fn a_saturday_game_after_a_thursday_tie_moves_to_sunday() {
        let home = [d(9, 12), d(9, 17), d(9, 26)];
        let away = [d(9, 12), d(9, 26)];
        assert_eq!(
            FixtureRest::best_day(d(9, 19), d(9, 1), &home, &away),
            d(9, 20)
        );
    }

    #[test]
    fn a_saturday_game_before_a_tuesday_tie_stays() {
        let home = [d(9, 12), d(9, 22), d(9, 26)];
        let away = [d(9, 12), d(9, 26)];
        assert_eq!(
            FixtureRest::best_day(d(9, 19), d(9, 1), &home, &away),
            d(9, 19)
        );
    }

    #[test]
    fn a_cup_tie_on_a_continental_night_goes_to_the_next_free_midweek() {
        let home = [d(9, 19), d(9, 23), d(9, 26), d(10, 3), d(10, 7)];
        let away = [d(9, 19), d(9, 26), d(10, 3)];
        assert_eq!(
            FixtureRest::best_day(d(9, 23), d(9, 1), &home, &away),
            d(9, 29)
        );
    }

    #[test]
    fn the_opponents_own_thursday_tie_is_respected() {
        let home = [d(9, 12), d(9, 15), d(9, 26)];
        let away = [d(9, 12), d(9, 17), d(9, 26)];
        let day = FixtureRest::best_day(d(9, 19), d(9, 1), &home, &away);
        assert_eq!(day, d(9, 20));
        assert!(FixtureRest::is_rested(day, &home) && FixtureRest::is_rested(day, &away));
    }

    #[test]
    fn a_fixture_is_never_moved_into_the_past() {
        // A Monday commitment: the Friday before would do, but it has gone.
        let home = [d(9, 21)];
        assert_eq!(
            FixtureRest::best_day(d(9, 19), d(9, 1), &home, &[]),
            d(9, 18)
        );
        let day = FixtureRest::best_day(d(9, 19), d(9, 19), &home, &[]);
        assert!(day >= d(9, 19));
        assert_eq!(day, d(9, 24));
    }

    #[test]
    fn a_window_day_is_never_proposed_even_when_nearest() {
        // The October 2027 window closes on Tuesday 12 October. With the
        // home side busy on Friday 15th, that Tuesday is the nearest day
        // rested from it; the fixture goes to Monday 18th instead.
        let d = |m: u32, day: u32| NaiveDate::from_ymd_opt(2027, m, day).unwrap();
        let home = [d(10, 15)];
        let day = FixtureRest::best_day(d(10, 13), d(10, 1), &home, &[]);
        assert_eq!(day, d(10, 18));
        assert!(InternationalCalendar::window_on(day).is_none());
    }

    #[test]
    fn a_window_in_the_way_does_not_shrink_the_search() {
        // Both sides play Thursday nights on 17 September and 1 October and
        // league games either side, so every day from 19 September to
        // 4 October is short of rest; the October 2026 window (5-13) blocks
        // the days after. The first rested day is Wednesday 14 October.
        let busy = [
            d(9, 12),
            d(9, 17),
            d(9, 22),
            d(9, 26),
            d(10, 1),
            d(10, 4),
            d(10, 22),
        ];
        assert_eq!(
            FixtureRest::best_day(d(9, 29), d(9, 13), &busy, &busy),
            d(10, 14)
        );
    }

    #[test]
    fn with_no_rested_day_the_largest_gap_wins() {
        // Busy every day of three weeks but for a hole around the 25th.
        let nominal = d(9, 19);
        let home: Vec<NaiveDate> = (-12..=12)
            .filter(|k| !(5..=7).contains(k))
            .map(|k| nominal + Duration::days(k))
            .collect();
        assert_eq!(
            FixtureRest::best_day(nominal, d(9, 1), &home, &[]),
            nominal + Duration::days(6)
        );
    }
}
