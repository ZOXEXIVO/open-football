use chrono::NaiveDate;
use std::collections::{HashMap, HashSet};

use super::competition::GroupFixture;
use super::config::ScheduleConfig;

/// The dates each national side (by country id) already has a fixture on,
/// for one team level.
pub type NationalBookings = HashMap<u32, HashSet<NaiveDate>>;

/// A qualifying group's fixture list: a double round-robin in which every
/// round has its own date.
pub struct QualifyingSchedule;

impl QualifyingSchedule {
    /// Circle-method double round-robin as `(round, home_idx, away_idx)`,
    /// rounds 1-based. Every side plays at most once a round; with an odd
    /// team count one side sits each round out.
    pub fn round_robin(team_count: usize) -> Vec<(u8, usize, usize)> {
        if team_count < 2 {
            return Vec::new();
        }

        // The circle method needs an even number of seats; the extra one
        // is the bye.
        let n = if team_count.is_multiple_of(2) {
            team_count
        } else {
            team_count + 1
        };
        let rounds_single = n - 1;

        let mut fixtures = Vec::new();
        let mut rotation: Vec<usize> = (0..n).collect();

        for round in 0..rounds_single {
            for i in 0..n / 2 {
                let home = rotation[i];
                let away = rotation[n - 1 - i];
                if home >= team_count || away >= team_count {
                    continue;
                }

                let md = (round + 1) as u8;
                if round % 2 == 0 {
                    fixtures.push((md, home, away));
                } else {
                    fixtures.push((md, away, home));
                }
            }

            // Fix seat 0, rotate the rest clockwise.
            let last = rotation[n - 1];
            for i in (2..n).rev() {
                rotation[i] = rotation[i - 1];
            }
            rotation[1] = last;
        }

        // Return legs: venues swapped, rounds after the first leg's.
        let first_leg_count = fixtures.len();
        for i in 0..first_leg_count {
            let (md, home, away) = fixtures[i];
            fixtures.push((md + rounds_single as u8, away, home));
        }

        fixtures
    }

    /// Date every round of a group on the qualifying calendar: each round on
    /// the first calendar date after the previous round's on which none of
    /// the group's sides is already booked, so no side ever plays twice on
    /// a date.
    pub fn group_fixtures(
        team_country_ids: &[u32],
        start_year: i32,
        schedule: &ScheduleConfig,
        booked: &NationalBookings,
    ) -> Vec<GroupFixture> {
        let round_robin = Self::round_robin(team_country_ids.len());
        let rounds = round_robin.iter().map(|f| f.0).max().unwrap_or(0) as usize;

        let free = |date: &NaiveDate| {
            team_country_ids
                .iter()
                .all(|id| booked.get(id).is_none_or(|dates| !dates.contains(date)))
        };
        let mut calendar = schedule.qualifying_calendar(start_year);
        let round_dates: Vec<NaiveDate> = (0..rounds).filter_map(|_| calendar.find(free)).collect();

        round_robin
            .into_iter()
            .filter_map(|(round, home_idx, away_idx)| {
                round_dates
                    .get(round as usize - 1)
                    .map(|date| GroupFixture {
                        matchday: round,
                        date: *date,
                        home_country_id: team_country_ids[home_idx],
                        away_country_id: team_country_ids[away_idx],
                        result: None,
                    })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::continent::national::config::ScheduleDate;

    fn qd(month: u32, day: u32, year_offset: i32) -> ScheduleDate {
        ScheduleDate {
            month,
            day,
            year_offset,
        }
    }

    /// The shipped qualifying calendar: four double-header windows.
    fn eight_dates() -> ScheduleConfig {
        ScheduleConfig {
            qualifying_dates: vec![
                qd(9, 6, 0),
                qd(9, 9, 0),
                qd(10, 11, 0),
                qd(10, 14, 0),
                qd(11, 15, 0),
                qd(11, 18, 0),
                qd(3, 22, 1),
                qd(3, 25, 1),
            ],
            tournament_group_dates: Vec::new(),
            tournament_knockout_dates: Vec::new(),
        }
    }

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn at_most_once_per_date(fixtures: &[GroupFixture]) {
        let mut seen: HashSet<(u32, NaiveDate)> = HashSet::new();
        for f in fixtures {
            for side in [f.home_country_id, f.away_country_id] {
                assert!(
                    seen.insert((side, f.date)),
                    "side {side} twice on {}",
                    f.date
                );
            }
        }
    }

    #[test]
    fn a_six_team_round_robin_keeps_all_ten_rounds() {
        let fixtures = QualifyingSchedule::round_robin(6);
        assert_eq!(fixtures.len(), 30);
        for round in 1..=10u8 {
            let mut sides: Vec<usize> = fixtures
                .iter()
                .filter(|f| f.0 == round)
                .flat_map(|f| [f.1, f.2])
                .collect();
            let n = sides.len();
            sides.sort_unstable();
            sides.dedup();
            assert_eq!(sides.len(), n, "round {round}: a side plays twice");
            assert_eq!(n, 6, "round {round}: every side plays");
        }
    }

    #[test]
    fn a_five_team_group_runs_into_the_next_septembers_window() {
        let fixtures = QualifyingSchedule::group_fixtures(
            &[1, 2, 3, 4, 5],
            2026,
            &eight_dates(),
            &NationalBookings::new(),
        );
        assert_eq!(fixtures.len(), 20);
        at_most_once_per_date(&fixtures);

        let mut dates: Vec<NaiveDate> = fixtures.iter().map(|f| f.date).collect();
        dates.sort_unstable();
        dates.dedup();
        assert_eq!(dates.len(), 10, "one date per round");
        assert_eq!(dates[0], d(2026, 9, 3));
        assert_eq!(dates[7], d(2027, 3, 28));
        assert_eq!(&dates[8..], &[d(2027, 9, 2), d(2027, 9, 5)]);

        // Rounds keep their order on the calendar.
        for pair in fixtures.windows(2) {
            if pair[0].matchday < pair[1].matchday {
                assert!(pair[0].date < pair[1].date);
            }
        }
    }

    #[test]
    fn a_side_booked_elsewhere_pushes_the_groups_rounds_on() {
        let mut booked = NationalBookings::new();
        booked.insert(3, [d(2026, 9, 3), d(2026, 9, 6)].into_iter().collect());

        let fixtures =
            QualifyingSchedule::group_fixtures(&[1, 2, 3, 4], 2026, &eight_dates(), &booked);
        at_most_once_per_date(&fixtures);
        let first = fixtures.iter().map(|f| f.date).min().unwrap();
        assert_eq!(first, d(2026, 10, 8), "round one waits for side 3");
        assert!(
            fixtures.iter().all(|f| !booked[&3].contains(&f.date)),
            "no round on a date side 3 is already booked"
        );
    }

    #[test]
    fn the_shipped_dates_are_each_windows_thursday_and_sunday() {
        let dates: Vec<NaiveDate> = eight_dates().qualifying_calendar(2027).take(8).collect();
        assert_eq!(
            dates,
            vec![
                d(2027, 9, 2),
                d(2027, 9, 5),
                d(2027, 10, 7),
                d(2027, 10, 10),
                d(2027, 11, 11),
                d(2027, 11, 14),
                d(2028, 3, 23),
                d(2028, 3, 26),
            ]
        );
    }

    #[test]
    fn qualifying_dates_stay_inside_their_windows_in_order() {
        use crate::InternationalCalendar;
        for start_year in 2026..=2060 {
            let dates: Vec<NaiveDate> =
                eight_dates().qualifying_calendar(start_year).take(16).collect();
            for date in &dates {
                assert!(
                    InternationalCalendar::window_on(*date).is_some(),
                    "{date} outside every window"
                );
            }
            for pair in dates.windows(2) {
                assert!(pair[0] < pair[1], "{} before {}", pair[1], pair[0]);
            }
        }
    }

    #[test]
    fn an_empty_calendar_schedules_nothing() {
        let config = ScheduleConfig {
            qualifying_dates: Vec::new(),
            tournament_group_dates: Vec::new(),
            tournament_knockout_dates: Vec::new(),
        };
        assert!(
            QualifyingSchedule::group_fixtures(
                &[1, 2, 3, 4],
                2026,
                &config,
                &NationalBookings::new()
            )
            .is_empty()
        );
    }
}
