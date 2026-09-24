use crate::InternationalCalendar;
use crate::continent::ContinentalMatchweek;
use crate::league::CompetitionLevel;
use chrono::{Datelike, Duration, NaiveDate};

/// The days a league plays its rounds: its round weekday in every week the
/// international calendar leaves free, from the first round to the closing
/// day. Midweek rounds are added only when those weekends run short, spread
/// evenly through the season's window-free weeks, and through the weeks
/// without continental football first, so a club in Europe is not asked to
/// play Tuesday and Thursday. Only a season window too small even for those
/// runs on past the closing day.
pub struct RoundCalendar;

impl RoundCalendar {
    pub fn dates(
        first_round: NaiveDate,
        closing_day: NaiveDate,
        rounds: usize,
        level: CompetitionLevel,
    ) -> Vec<NaiveDate> {
        let weekends: Vec<NaiveDate> = Self::weekly_from(first_round)
            .take_while(|day| *day <= closing_day)
            .filter(|day| InternationalCalendar::window_on(*day).is_none())
            .collect();
        if weekends.len() >= rounds {
            return weekends[..rounds].to_vec();
        }

        let need = rounds - weekends.len();
        let (quiet, continental): (Vec<NaiveDate>, Vec<NaiveDate>) =
            Self::midweek_candidates(&weekends, level)
                .into_iter()
                .partition(|day| !ContinentalMatchweek::is_matchweek(*day));
        let after_quiet = need.saturating_sub(quiet.len());
        let overflow = after_quiet.saturating_sub(continental.len());

        let mut dates = weekends;
        dates.extend(Self::spread(&quiet, need));
        dates.extend(Self::spread(&continental, after_quiet));
        dates.extend(
            Self::weekly_from(first_round)
                .skip_while(|day| *day <= closing_day)
                .filter(|day| InternationalCalendar::window_on(*day).is_none())
                .take(overflow),
        );
        dates.sort_unstable();
        dates
    }

    /// `count` of `candidates`, evenly spaced; all of them when short.
    fn spread(candidates: &[NaiveDate], count: usize) -> Vec<NaiveDate> {
        let m = candidates.len();
        if count >= m {
            return candidates.to_vec();
        }
        (0..count)
            .map(|i| candidates[(2 * i + 1) * m / (2 * count)])
            .collect()
    }

    fn weekly_from(first: NaiveDate) -> impl Iterator<Item = NaiveDate> {
        (0..).map(move |week| first + Duration::weeks(week))
    }

    /// The midweek day of every window-free week strictly between the first
    /// and the last weekend round.
    fn midweek_candidates(weekends: &[NaiveDate], level: CompetitionLevel) -> Vec<NaiveDate> {
        let (Some(first), Some(last)) = (weekends.first(), weekends.last()) else {
            return Vec::new();
        };
        let monday_of =
            |day: NaiveDate| day - Duration::days(day.weekday().num_days_from_monday() as i64);
        let midweek = level.midweek_weekday().num_days_from_monday() as i64;
        Self::weekly_from(monday_of(*first) + Duration::weeks(1))
            .take_while(|monday| *monday <= monday_of(*last))
            .filter(|monday| {
                (0..7).all(|d| {
                    InternationalCalendar::window_on(*monday + Duration::days(d)).is_none()
                })
            })
            .map(|monday| monday + Duration::days(midweek))
            .filter(|day| day > first && day < last)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::DateUtils;
    use chrono::Weekday;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn first_saturday_from(date: NaiveDate) -> NaiveDate {
        DateUtils::next_weekday(date, Weekday::Sat)
    }

    #[test]
    fn a_20_club_league_fits_every_season_on_saturdays_and_tuesdays() {
        for year in 2026..=2060 {
            let first = first_saturday_from(d(year, 8, 16));
            let close = d(year + 1, 5, 25);
            let dates = RoundCalendar::dates(first, close, 38, CompetitionLevel::Senior);
            assert_eq!(dates.len(), 38, "{year}");
            for day in &dates {
                assert!(
                    matches!(day.weekday(), Weekday::Sat | Weekday::Tue),
                    "{year}: round on {day}"
                );
                assert!(
                    InternationalCalendar::window_on(*day).is_none(),
                    "{year}: {day}"
                );
                assert!(*day <= close, "{year}: {day} after the close");
            }
            for pair in dates.windows(2) {
                assert!(
                    pair[1] - pair[0] >= Duration::days(3),
                    "{year}: {} then {}",
                    pair[0],
                    pair[1]
                );
            }
        }
    }

    #[test]
    fn the_2027_28_season_needs_exactly_two_tuesdays() {
        let first = first_saturday_from(d(2027, 8, 16));
        let dates = RoundCalendar::dates(first, d(2028, 5, 25), 38, CompetitionLevel::Senior);
        let tuesdays = dates
            .iter()
            .filter(|day| day.weekday() == Weekday::Tue)
            .count();
        assert_eq!(tuesdays, 2);
    }

    #[test]
    fn midweek_rounds_keep_out_of_continental_weeks_when_there_is_room() {
        for year in 2026..=2060 {
            let first = first_saturday_from(d(year, 8, 16));
            let dates =
                RoundCalendar::dates(first, d(year + 1, 5, 25), 38, CompetitionLevel::Senior);
            for day in dates.iter().filter(|day| day.weekday() == Weekday::Tue) {
                assert!(
                    !ContinentalMatchweek::is_matchweek(*day),
                    "{year}: a Tuesday round on {day} in a continental week"
                );
            }
        }
    }

    #[test]
    fn a_24_club_league_ends_by_its_may_close() {
        for year in 2026..=2060 {
            let first = first_saturday_from(d(year, 8, 9));
            let close = d(year + 1, 5, 3);
            let dates = RoundCalendar::dates(first, close, 46, CompetitionLevel::Senior);
            assert_eq!(dates.len(), 46, "{year}");
            assert!(
                dates.iter().all(|day| *day <= close),
                "{year}: past the close"
            );
            assert!(
                dates
                    .iter()
                    .all(|day| InternationalCalendar::window_on(*day).is_none()),
                "{year}: a round inside a window"
            );
        }
    }

    #[test]
    fn a_league_with_slack_plays_only_on_saturdays() {
        let first = first_saturday_from(d(2027, 8, 7));
        let dates = RoundCalendar::dates(first, d(2028, 5, 31), 34, CompetitionLevel::Senior);
        assert_eq!(dates.len(), 34);
        assert!(dates.iter().all(|day| day.weekday() == Weekday::Sat));
        let skipped = [d(2027, 9, 4), d(2027, 10, 9), d(2027, 11, 13)];
        assert!(
            dates.iter().all(|day| !skipped.contains(day)),
            "window Saturdays"
        );
        assert_eq!(dates[0], d(2027, 8, 7));
        assert_eq!(dates[1], d(2027, 8, 14));
    }

    #[test]
    fn a_window_too_small_still_schedules_every_round() {
        let first = first_saturday_from(d(2027, 8, 7));
        let close = d(2027, 11, 30);
        let dates = RoundCalendar::dates(first, close, 46, CompetitionLevel::Senior);
        assert_eq!(dates.len(), 46);
        assert!(
            dates
                .iter()
                .all(|day| InternationalCalendar::window_on(*day).is_none()),
            "no round inside a window"
        );
        assert!(
            dates
                .iter()
                .filter(|day| **day > close)
                .all(|day| day.weekday() == Weekday::Sat),
            "overflow rounds stay on the round weekday"
        );
        for pair in dates.windows(2) {
            assert!(pair[0] < pair[1]);
        }
    }

    #[test]
    fn a_development_league_plays_fridays_and_wednesdays_only_when_short() {
        let first = DateUtils::next_weekday(d(2027, 8, 1), Weekday::Fri);
        let close = d(2028, 5, 31);
        let roomy = RoundCalendar::dates(first, close, 30, CompetitionLevel::Development);
        assert!(roomy.iter().all(|day| day.weekday() == Weekday::Fri));

        let tight = RoundCalendar::dates(first, d(2028, 4, 30), 46, CompetitionLevel::Development);
        assert_eq!(tight.len(), 46);
        for day in &tight {
            assert!(
                matches!(day.weekday(), Weekday::Fri | Weekday::Wed),
                "{day}"
            );
            assert!(InternationalCalendar::window_on(*day).is_none(), "{day}");
        }
        assert!(tight.iter().any(|day| day.weekday() == Weekday::Wed));
    }
}
