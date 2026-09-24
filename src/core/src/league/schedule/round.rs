use crate::league::{
    CompetitionLevel, KickoffClock, LeagueSettings, RoundCalendar, ScheduleError,
    ScheduleGenerator, ScheduleItem, ScheduleTour, Season,
};
use crate::utils::DateUtils;
use chrono::{Datelike, NaiveDate};
use log::warn;

pub struct RoundSchedule;

impl Default for RoundSchedule {
    fn default() -> Self {
        Self::new()
    }
}

impl RoundSchedule {
    /// Bye seat for odd team counts. Real team ids are allocated
    /// sequentially and never reach it.
    const BYE: u32 = u32::MAX;

    pub fn new() -> Self {
        RoundSchedule {}
    }

    /// One tour per date in `round_days`, in order.
    fn tours(
        league_id: u32,
        league_slug: String,
        teams: &[u32],
        round_days: &[NaiveDate],
        level: CompetitionLevel,
    ) -> Vec<ScheduleTour> {
        if teams.len() < 2 {
            return Vec::new();
        }

        // Odd team counts carry a bye seat, so every round keeps the same
        // stride of slots; the bye pair is skipped when building tours.
        let games_per_round = teams.len().div_ceil(2);

        let mut result = Vec::with_capacity(round_days.len());
        let games = Self::game_pairs(teams);
        if games.is_empty() {
            return result;
        }

        for (tour_idx, round_day) in round_days.iter().enumerate() {
            let mut tour = ScheduleTour::new((tour_idx + 1) as u8, games_per_round);
            let games_offset = tour_idx * games_per_round;

            for game_idx in 0..games_per_round {
                let pos = games_offset + game_idx;
                if pos >= games.len() {
                    break;
                }
                let (home_team_id, away_team_id) = games[pos];
                if home_team_id == Self::BYE || away_team_id == Self::BYE {
                    continue;
                }
                let kickoff = KickoffClock::at(*round_day, level, tour.items.len());
                tour.items.push(ScheduleItem::new(
                    league_id,
                    String::from(&league_slug),
                    home_team_id,
                    away_team_id,
                    kickoff,
                    None,
                ));
            }

            result.push(tour);
        }

        result
    }

    /// A double round-robin's rounds; an odd team count adds a bye seat, so
    /// every team sits one round out in each half.
    fn round_count(team_count: usize) -> usize {
        if team_count.is_multiple_of(2) {
            team_count.saturating_sub(1) * 2
        } else {
            team_count * 2
        }
    }

    /// The first `month`/`day` on or after `from`.
    fn on_or_after(from: NaiveDate, month: u8, day: u8) -> NaiveDate {
        let on = |year: i32| NaiveDate::from_ymd_opt(year, month as u32, day as u32).unwrap();
        let this_year = on(from.year());
        if this_year >= from {
            this_year
        } else {
            on(from.year() + 1)
        }
    }

    /// Double round-robin via the circle method: every team plays every
    /// other team exactly twice, once at home and once away. Odd team counts
    /// seat a bye; its pairs stay in the sequence as `(BYE, BYE)` so the
    /// per-round stride stays fixed, and `tours` skips them.
    fn game_pairs(teams: &[u32]) -> Vec<(u32, u32)> {
        let n = teams.len();
        if n < 2 {
            return Vec::new();
        }

        let mut seats: Vec<u32> = teams.to_vec();
        if !n.is_multiple_of(2) {
            seats.push(Self::BYE);
        }
        let seats_len = seats.len();
        let half = seats_len / 2;
        let rounds_per_half = seats_len - 1;

        let mut first_half: Vec<(u32, u32)> = Vec::with_capacity(rounds_per_half * half);
        for round in 0..rounds_per_half {
            // Home side alternates on the round alone: a `(round + seat)`
            // parity cancels with the seat rotation and strings a team's
            // home games together.
            let top_is_home = round % 2 == 0;
            for i in 0..half {
                let top = seats[i];
                let bottom = seats[seats_len - 1 - i];
                let (home, away) = if top_is_home {
                    (top, bottom)
                } else {
                    (bottom, top)
                };
                first_half.push((home, away));
            }
            // Rotate seats 1..n clockwise; seat 0 is fixed.
            let last = seats[seats_len - 1];
            for j in (2..seats_len).rev() {
                seats[j] = seats[j - 1];
            }
            seats[1] = last;
        }

        let mut result = Vec::with_capacity(first_half.len() * 2);
        result.extend(first_half.iter().map(|&(h, a)| {
            if h == Self::BYE || a == Self::BYE {
                (Self::BYE, Self::BYE)
            } else {
                (h, a)
            }
        }));
        for &(h, a) in &first_half {
            if h == Self::BYE || a == Self::BYE {
                result.push((Self::BYE, Self::BYE));
            } else {
                result.push((a, h));
            }
        }
        result
    }
}

impl ScheduleGenerator for RoundSchedule {
    fn generate(
        &self,
        league_id: u32,
        league_slug: &str,
        season: Season,
        teams: &[u32],
        league_settings: &LeagueSettings,
        level: CompetitionLevel,
    ) -> Result<Vec<ScheduleTour>, ScheduleError> {
        let teams_len = teams.len();

        if teams_len == 0 {
            warn!("schedule: team_len is empty. skip generation");
            ScheduleError::new("team_len is empty");
        }

        let round_weekday = level.round_weekday();
        let starting = &league_settings.season_starting_half;
        let ending = &league_settings.season_ending_half;
        let opening = NaiveDate::from_ymd_opt(
            season.start_year as i32,
            starting.from_month as u32,
            starting.from_day as u32,
        )
        .unwrap();
        let rounds = Self::round_count(teams_len);

        // The first round day on or after the window opens — never before
        // it, since the schedule is generated on the opening day itself.
        let first_round = DateUtils::next_weekday(opening, round_weekday);

        let plan = |first: NaiveDate, closing: NaiveDate, rounds: usize| {
            let days = RoundCalendar::dates(first, closing, rounds, level);
            let late = days.iter().filter(|day| **day > closing).count();
            if late > 0 {
                warn!("schedule: {league_slug} plays {late} of {rounds} rounds after {closing}");
            }
            days
        };

        // Split seasons (Apertura/Clausura) play the mirrored second
        // round-robin inside the second tournament's own window.
        let round_days = if league_settings.split_season {
            let first_close = Self::on_or_after(opening, starting.to_month, starting.to_day);
            let mut days = plan(first_round, first_close, rounds / 2);

            let second_open = Self::on_or_after(opening, ending.from_month, ending.from_day);
            let second_close = Self::on_or_after(second_open, ending.to_month, ending.to_day);
            let after_first = days.last().map_or(second_open, |last| last.succ_opt().unwrap());
            let second_round = DateUtils::next_weekday(second_open.max(after_first), round_weekday);
            days.extend(plan(second_round, second_close, rounds - rounds / 2));
            days
        } else {
            let closing = Self::on_or_after(opening, ending.to_month, ending.to_day);
            plan(first_round, closing, rounds)
        };

        Ok(Self::tours(
            league_id,
            String::from(league_slug),
            teams,
            &round_days,
            level,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::league::DayMonthPeriod;

    #[test]
    fn double_round_robin_every_pair_exactly_once_for_20_teams() {
        use std::collections::HashMap;
        let teams: Vec<u32> = (1..=20).collect();
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 1, 6),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
            split_season: false,
        };
        let tours = RoundSchedule::new()
            .generate(
                1,
                "t",
                Season::new(2026),
                &teams,
                &settings,
                CompetitionLevel::Senior,
            )
            .unwrap();

        let mut home_count: HashMap<u32, u32> = HashMap::new();
        let mut away_count: HashMap<u32, u32> = HashMap::new();
        let mut pair_count: HashMap<(u32, u32), u32> = HashMap::new();
        for tour in &tours {
            for item in &tour.items {
                *home_count.entry(item.home_team_id).or_default() += 1;
                *away_count.entry(item.away_team_id).or_default() += 1;
                *pair_count
                    .entry((item.home_team_id, item.away_team_id))
                    .or_default() += 1;
            }
        }

        // Every team: 19 home, 19 away in a 20-team double round-robin.
        for &t in &teams {
            assert_eq!(
                home_count.get(&t).copied().unwrap_or(0),
                19,
                "home count team {}",
                t
            );
            assert_eq!(
                away_count.get(&t).copied().unwrap_or(0),
                19,
                "away count team {}",
                t
            );
        }

        // Every ordered (home, away) pair exactly once (380 total).
        let mut seen = 0;
        for (&(h, a), &c) in &pair_count {
            assert_ne!(h, a, "team plays itself");
            assert_eq!(c, 1, "pair {}->{} count {}", h, a, c);
            seen += 1;
        }
        assert_eq!(seen, 20 * 19);
    }

    #[test]
    fn schedule_has_no_absurd_home_away_streaks() {
        // The rotation bug produced "team X plays 10 consecutive home
        // games to start the season" because the home/away parity
        // cancelled with seat rotation. A correct schedule keeps runs
        // short — real competitions rarely exceed 3 in a row.
        let teams: Vec<u32> = (1..=20).collect();
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 1, 6),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
            split_season: false,
        };
        let tours = RoundSchedule::new()
            .generate(
                1,
                "t",
                Season::new(2026),
                &teams,
                &settings,
                CompetitionLevel::Senior,
            )
            .unwrap();

        for &tid in &teams {
            let mut sequence: Vec<bool> = Vec::new(); // true = home
            for tour in &tours {
                for item in &tour.items {
                    if item.home_team_id == tid {
                        sequence.push(true);
                    } else if item.away_team_id == tid {
                        sequence.push(false);
                    }
                }
            }
            let mut longest_home = 0;
            let mut longest_away = 0;
            let mut cur_home = 0;
            let mut cur_away = 0;
            for &is_home in &sequence {
                if is_home {
                    cur_home += 1;
                    cur_away = 0;
                } else {
                    cur_away += 1;
                    cur_home = 0;
                }
                longest_home = longest_home.max(cur_home);
                longest_away = longest_away.max(cur_away);
            }
            assert!(
                longest_home <= 4,
                "team {} had {} consecutive home games",
                tid,
                longest_home
            );
            assert!(
                longest_away <= 4,
                "team {} had {} consecutive away games",
                tid,
                longest_away
            );
        }
    }

    #[test]
    fn schedule_handles_odd_team_count_with_byes() {
        use std::collections::HashMap;
        let teams: Vec<u32> = (1..=15).collect();
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 1, 6),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
            split_season: false,
        };
        let tours = RoundSchedule::new()
            .generate(
                1,
                "t",
                Season::new(2026),
                &teams,
                &settings,
                CompetitionLevel::Senior,
            )
            .unwrap();

        // Odd team counts produce a bye slot per round; `tours`
        // drops those before they reach a tour. Total matches should be
        // n * (n-1) = 15 * 14 = 210, split across ~30 rounds.
        let mut home_count: HashMap<u32, u32> = HashMap::new();
        let mut away_count: HashMap<u32, u32> = HashMap::new();
        let mut pair_count: HashMap<(u32, u32), u32> = HashMap::new();
        for tour in &tours {
            for item in &tour.items {
                *home_count.entry(item.home_team_id).or_default() += 1;
                *away_count.entry(item.away_team_id).or_default() += 1;
                *pair_count
                    .entry((item.home_team_id, item.away_team_id))
                    .or_default() += 1;
            }
        }

        for &t in &teams {
            assert_eq!(
                home_count.get(&t).copied().unwrap_or(0),
                14,
                "home count team {}",
                t
            );
            assert_eq!(
                away_count.get(&t).copied().unwrap_or(0),
                14,
                "away count team {}",
                t
            );
        }
        for (&(h, a), &c) in &pair_count {
            assert_ne!(h, a);
            assert!(h != u32::MAX && a != u32::MAX, "bye should not leak");
            assert_eq!(c, 1, "pair {}->{} count {}", h, a, c);
        }
        assert_eq!(pair_count.len(), 15 * 14);
    }

    #[test]
    fn split_season_second_tournament_starts_in_its_own_window() {
        use crate::InternationalCalendar;
        use std::collections::HashMap;
        // Argentine shape: 15 teams, Apertura Feb-Jun, Clausura from Jul 15.
        let teams: Vec<u32> = (1..=15).collect();
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 2, 30, 6),
            season_ending_half: DayMonthPeriod::new(15, 7, 15, 12),
            tier: 1,
            promotion_spots: 0,
            relegation_spots: 1,
            league_group: None,
            split_season: true,
        };
        let tours = RoundSchedule::new()
            .generate(
                1,
                "t",
                Season::new(2026),
                &teams,
                &settings,
                CompetitionLevel::Senior,
            )
            .unwrap();

        assert_eq!(tours.len(), 30, "two 15-round single round-robins");

        let apertura_close = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
        let clausura_start = NaiveDate::from_ymd_opt(2026, 7, 15).unwrap();
        let clausura_close = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        for (idx, tour) in tours.iter().enumerate() {
            let date = tour.items.first().map(|i| i.date.date()).unwrap();
            assert!(
                InternationalCalendar::window_on(date).is_none(),
                "tour {} on {} inside an international window",
                idx + 1,
                date
            );
            if idx < 15 {
                assert!(
                    date <= apertura_close,
                    "Apertura tour {} on {} after the Apertura closes",
                    idx + 1,
                    date
                );
            } else {
                assert!(
                    date >= clausura_start && date <= clausura_close,
                    "Clausura tour {} on {} outside July 15 - December 15",
                    idx + 1,
                    date
                );
            }
        }

        // Each half is a full single round-robin: 7 games per team per
        // half... i.e. every team plays 14 games per half (7 home+away mix)
        // and meets every opponent once.
        let mut first_half_games: HashMap<u32, u32> = HashMap::new();
        for tour in &tours[..15] {
            for item in &tour.items {
                *first_half_games.entry(item.home_team_id).or_default() += 1;
                *first_half_games.entry(item.away_team_id).or_default() += 1;
            }
        }
        for &t in &teams {
            assert_eq!(
                first_half_games.get(&t).copied().unwrap_or(0),
                14,
                "team {} games in the Apertura",
                t
            );
        }
    }

    #[test]
    fn generate_schedule_is_correct() {
        let schedule = RoundSchedule::new();

        const LEAGUE_ID: u32 = 1;

        let teams = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

        let league_settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 1, 30, 6),
            season_ending_half: DayMonthPeriod::new(1, 7, 1, 12),
            tier: 0,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
            split_season: false,
        };

        let schedule_tours = schedule
            .generate(
                LEAGUE_ID,
                "slug",
                Season::new(2020),
                &teams,
                &league_settings,
                CompetitionLevel::Senior,
            )
            .unwrap();

        assert_eq!(30, schedule_tours.len());

        for tour in &schedule_tours {
            for team_id in &teams {
                let home_team_id = tour
                    .items
                    .iter()
                    .map(|t| t.home_team_id)
                    .filter(|t| *t == *team_id)
                    .count();
                assert!(
                    home_team_id < 2,
                    "multiple home_team {} in tour {}",
                    team_id,
                    tour.num
                );

                let away_team_id = tour
                    .items
                    .iter()
                    .map(|t| t.away_team_id)
                    .filter(|t| *t == *team_id)
                    .count();
                assert!(
                    away_team_id < 2,
                    "multiple away_team {} in tour {}",
                    team_id,
                    tour.num
                );
            }
        }
    }

    fn august_settings() -> LeagueSettings {
        LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
            tier: 1,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
            split_season: false,
        }
    }

    fn fixtures(level: CompetitionLevel, teams: &[u32]) -> Vec<ScheduleItem> {
        RoundSchedule::new()
            .generate(1, "t", Season::new(2026), teams, &august_settings(), level)
            .unwrap()
            .into_iter()
            .flat_map(|t| t.items)
            .collect()
    }

    #[test]
    fn senior_rounds_are_saturday_afternoons_never_midnight() {
        use chrono::{Datelike, Timelike, Weekday};
        let teams: Vec<u32> = (1..=18).collect();
        for item in fixtures(CompetitionLevel::Senior, &teams) {
            assert_eq!(item.date.weekday(), Weekday::Sat, "{}", item.date);
            assert!(
                (12..=21).contains(&item.date.hour()),
                "senior kickoff {}",
                item.date
            );
        }
    }

    #[test]
    fn development_rounds_are_fridays() {
        use chrono::{Datelike, Timelike, Weekday};
        let teams: Vec<u32> = (101..=118).collect();
        for item in fixtures(CompetitionLevel::Development, &teams) {
            assert_eq!(item.date.weekday(), Weekday::Fri, "{}", item.date);
            assert!(
                (10..=14).contains(&item.date.hour()),
                "youth kickoff {}",
                item.date
            );
        }
    }

    #[test]
    fn league_rounds_stop_for_international_windows() {
        use crate::InternationalCalendar;
        let items = fixtures(CompetitionLevel::Senior, &(1..=20).collect::<Vec<_>>());
        assert_eq!(items.len(), 380);
        for item in &items {
            assert!(
                InternationalCalendar::window_on(item.date.date()).is_none(),
                "senior fixture on {} inside a window",
                item.date
            );
        }
        let youth = fixtures(CompetitionLevel::Development, &(101..=120).collect::<Vec<_>>());
        for item in &youth {
            assert!(
                InternationalCalendar::window_on(item.date.date()).is_none(),
                "youth fixture on {} inside a window",
                item.date
            );
        }
    }

    #[test]
    fn a_crowded_senior_league_adds_tuesdays_and_still_ends_on_time() {
        use chrono::{Datelike, Weekday};
        // 24 clubs, 46 rounds, from 9 August to 3 May.
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(9, 8, 31, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 3, 5),
            ..august_settings()
        };
        let tours = RoundSchedule::new()
            .generate(
                1,
                "t",
                Season::new(2027),
                &(1..=24).collect::<Vec<_>>(),
                &settings,
                CompetitionLevel::Senior,
            )
            .unwrap();
        assert_eq!(tours.len(), 46);
        let close = NaiveDate::from_ymd_opt(2028, 5, 3).unwrap();
        let mut last = None;
        for tour in &tours {
            let day = tour.items[0].date.date();
            assert!(tour.items.iter().all(|i| i.date.date() == day), "one day per round");
            assert!(day <= close, "round {} on {day} after the close", tour.num);
            assert!(matches!(day.weekday(), Weekday::Sat | Weekday::Tue), "{day}");
            if let Some(previous) = last {
                assert!(day > previous, "round {} not after the one before", tour.num);
            }
            last = Some(day);
        }
    }

    #[test]
    fn saturday_season_start_puts_youth_round_one_on_the_next_friday() {
        // 1 Aug 2026 is a Saturday: the senior league opens that day, and
        // the Friday before it is already gone when the schedule is drawn.
        let senior = fixtures(CompetitionLevel::Senior, &[1, 2, 3, 4]);
        let youth = fixtures(CompetitionLevel::Development, &[101, 102, 103, 104]);
        let opening = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();

        let senior_first = senior.iter().map(|i| i.date.date()).min().unwrap();
        let youth_first = youth.iter().map(|i| i.date.date()).min().unwrap();
        assert_eq!(senior_first, opening);
        assert_eq!(youth_first, NaiveDate::from_ymd_opt(2026, 8, 7).unwrap());
        assert!(youth.iter().all(|i| i.date.date() >= opening));
    }

    #[test]
    fn a_youth_league_never_shares_a_day_with_its_parent_league() {
        use std::collections::HashSet;
        let senior_days: HashSet<NaiveDate> =
            fixtures(CompetitionLevel::Senior, &(1..=16).collect::<Vec<_>>())
                .iter()
                .map(|i| i.date.date())
                .collect();
        for item in fixtures(
            CompetitionLevel::Development,
            &(101..=116).collect::<Vec<_>>(),
        ) {
            assert!(
                !senior_days.contains(&item.date.date()),
                "youth fixture on senior day {}",
                item.date
            );
        }
    }
}
