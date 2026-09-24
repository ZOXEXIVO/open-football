use crate::context::GlobalContext;
use crate::league::round::RoundSchedule;
use crate::league::{
    CompetitionLevel, LeagueMatch, LeagueSettings, ScheduleGenerator, ScheduleResult, Season,
};
use crate::r#match::Score;
use chrono::{Datelike, NaiveDate, NaiveDateTime};
use log::error;

#[derive(Debug, Clone, Default)]
pub struct Schedule {
    pub tours: Vec<ScheduleTour>,
}

#[derive(Debug, Clone)]
pub struct ScheduleTour {
    pub num: u8,
    pub items: Vec<ScheduleItem>,
}

#[derive(Debug, Clone)]
pub struct ScheduleItem {
    pub id: String,

    pub league_id: u32,
    pub league_slug: String,

    pub date: NaiveDateTime,

    pub home_team_id: u32,
    pub away_team_id: u32,

    pub result: Option<Score>,
}

impl Schedule {
    pub fn new() -> Self {
        Schedule { tours: Vec::new() }
    }

    pub fn simulate(
        &mut self,
        league_settings: &LeagueSettings,
        level: CompetitionLevel,
        ctx: GlobalContext<'_>,
    ) -> ScheduleResult {
        let mut result = ScheduleResult::new();

        if self.tours.is_empty() || league_settings.is_time_for_new_schedule(&ctx.simulation) {
            let league_ctx = ctx.league.as_ref().unwrap();

            let generator = RoundSchedule::new();

            match generator.generate(
                league_ctx.id,
                &league_ctx.slug,
                Season::new(ctx.simulation.date.year() as u16),
                league_ctx.team_ids,
                league_settings,
                level,
            ) {
                Ok(generated_schedule) => {
                    self.tours = generated_schedule;
                    result.generated = true;
                }
                Err(error) => {
                    error!("Generating schedule error: {}", error.message);
                }
            }
        }

        result.scheduled_matches = self
            .get_matches(ctx.simulation.date)
            .iter()
            .map(|sm| LeagueMatch {
                id: sm.id.clone(),
                league_id: sm.league_id,
                league_slug: String::from(&sm.league_slug),
                date: sm.date,
                home_team_id: sm.home_team_id,
                away_team_id: sm.away_team_id,
                result: None,
                // League fixtures carry no knockout bracket position.
                cup_round: None,
                cup_total_rounds: None,
            })
            .collect();

        result
    }

    /// Every fixture kicking off on `date`'s calendar day. The simulation
    /// clock reads midnight while kickoffs are in the afternoon, so the
    /// match is by day, never by instant.
    pub fn get_matches(&self, date: NaiveDateTime) -> Vec<ScheduleItem> {
        let day = date.date();
        self.tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(|s| s.date.date() == day)
            .map(|s| ScheduleItem {
                result: None,
                ..s.clone()
            })
            .collect()
    }

    pub fn get_matches_for_team(&self, team_id: u32) -> Vec<ScheduleItem> {
        self.tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(|s| s.home_team_id == team_id || s.away_team_id == team_id)
            .cloned()
            .collect()
    }

    pub fn update_match_result(&mut self, id: &str, score: &Score) {
        let mut _updated = false;

        for tour in &mut self.tours.iter_mut().filter(|t| !t.played()) {
            if let Some(item) = tour.items.iter_mut().find(|i| i.id == id) {
                item.result = Some(score.clone());
                _updated = true;
            }
        }
    }

    /// Every team with a fixture on `date`, played or not — the continental
    /// drain asks after the domestic batch has already written its results.
    pub fn teams_playing_on(&self, date: NaiveDate) -> impl Iterator<Item = u32> + '_ {
        self.tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(move |i| i.date.date() == date)
            .flat_map(|i| [i.home_team_id, i.away_team_id])
    }

    /// Rearrange one fixture. Its id, round and sides stay; only the kickoff
    /// moves, and the round is kept in kickoff order for the pages that list
    /// it by day.
    pub fn move_fixture(&mut self, id: &str, kickoff: NaiveDateTime) {
        for tour in &mut self.tours {
            if let Some(item) = tour.items.iter_mut().find(|i| i.id == id) {
                item.date = kickoff;
                tour.items.sort_by_key(|i| i.date);
                return;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScheduleError {
    pub message: String,
}

impl ScheduleError {
    pub fn new(message: &'static str) -> Self {
        ScheduleError {
            message: message.to_owned(),
        }
    }
}

impl ScheduleItem {
    pub fn new(
        league_id: u32,
        league_slug: String,
        home_team_id: u32,
        away_team_id: u32,
        date: NaiveDateTime,
        result: Option<Score>,
    ) -> Self {
        let id = format!("{}_{}_{}", date.date(), home_team_id, away_team_id);

        ScheduleItem {
            id,
            league_id,
            league_slug,
            date,
            result,
            home_team_id,
            away_team_id,
        }
    }
}

impl ScheduleTour {
    pub fn new(num: u8, games_count: usize) -> Self {
        ScheduleTour {
            num,
            items: Vec::with_capacity(games_count),
        }
    }

    pub fn played(&self) -> bool {
        self.items.iter().all(|i| i.result.is_some())
    }

    pub fn start_date(&self) -> NaiveDate {
        self.items
            .iter()
            .min_by_key(|t| t.date)
            .unwrap()
            .date
            .date()
    }

    pub fn end_date(&self) -> NaiveDate {
        self.items
            .iter()
            .max_by_key(|t| t.date)
            .unwrap()
            .date
            .date()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#match::TeamScore;
    use chrono::NaiveDate;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    fn weekend_round() -> Schedule {
        let mut tour = ScheduleTour::new(1, 2);
        tour.items.push(ScheduleItem::new(
            1,
            "l".into(),
            1,
            2,
            at(2026, 10, 10, 15, 0),
            None,
        ));
        tour.items.push(ScheduleItem::new(
            1,
            "l".into(),
            3,
            4,
            at(2026, 10, 11, 16, 30),
            None,
        ));
        Schedule { tours: vec![tour] }
    }

    fn todays_fixtures(schedule: &mut Schedule, tick: NaiveDateTime) -> Vec<(u32, u32)> {
        use crate::context::{GlobalContext, SimulationContext};
        use crate::league::DayMonthPeriod;
        let settings = LeagueSettings {
            season_starting_half: DayMonthPeriod::new(1, 8, 30, 12),
            season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
            tier: 1,
            promotion_spots: 0,
            relegation_spots: 0,
            league_group: None,
            split_season: false,
        };
        let ctx = GlobalContext::new(SimulationContext::new(tick));
        schedule
            .simulate(&settings, CompetitionLevel::Senior, ctx)
            .scheduled_matches
            .iter()
            .map(|m| (m.home_team_id, m.away_team_id))
            .collect()
    }

    #[test]
    fn an_afternoon_kickoff_is_played_on_its_midnight_tick() {
        let mut schedule = weekend_round();
        let played = todays_fixtures(&mut schedule, at(2026, 10, 10, 0, 0));
        assert_eq!(played, vec![(1, 2)]);
    }

    #[test]
    fn a_round_over_two_days_plays_each_fixture_on_its_own_day() {
        let mut schedule = weekend_round();
        assert_eq!(
            todays_fixtures(&mut schedule, at(2026, 10, 10, 0, 0)),
            vec![(1, 2)]
        );
        assert_eq!(
            todays_fixtures(&mut schedule, at(2026, 10, 11, 0, 0)),
            vec![(3, 4)]
        );
        assert!(todays_fixtures(&mut schedule, at(2026, 10, 12, 0, 0)).is_empty());
    }

    #[test]
    fn teams_playing_on_counts_played_and_unplayed_fixtures_of_that_day() {
        let mut schedule = weekend_round();
        schedule.tours[0].items[0].result = Some(Score::new(1, 2));
        let saturday = NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
        let sunday = NaiveDate::from_ymd_opt(2026, 10, 11).unwrap();

        let mut sat: Vec<u32> = schedule.teams_playing_on(saturday).collect();
        sat.sort_unstable();
        assert_eq!(sat, vec![1, 2]);
        let mut sun: Vec<u32> = schedule.teams_playing_on(sunday).collect();
        sun.sort_unstable();
        assert_eq!(sun, vec![3, 4]);
    }

    #[test]
    fn a_moved_fixture_keeps_its_identity_and_the_round_stays_in_kickoff_order() {
        let mut tour = ScheduleTour::new(1, 3);
        for (home, away, hour) in [(1, 2, 13), (3, 4, 15), (5, 6, 17)] {
            tour.items.push(ScheduleItem::new(
                1,
                "l".into(),
                home,
                away,
                at(2026, 10, 10, hour, 0),
                None,
            ));
        }
        let mut schedule = Schedule { tours: vec![tour] };
        let id = schedule.tours[0].items[0].id.clone();

        schedule.move_fixture(&id, at(2026, 10, 11, 16, 30));

        let items = &schedule.tours[0].items;
        assert_eq!(items.len(), 3);
        let moved = items.last().unwrap();
        assert_eq!(moved.id, id, "the id survives the move");
        assert_eq!((moved.home_team_id, moved.away_team_id), (1, 2));
        assert_eq!(moved.date, at(2026, 10, 11, 16, 30));
        assert!(items.windows(2).all(|w| w[0].date <= w[1].date));

        // The day lookup and the result write both still find it.
        let sunday: Vec<ScheduleItem> = schedule.get_matches(at(2026, 10, 11, 0, 0));
        assert_eq!(sunday.len(), 1);
        assert_eq!(sunday[0].id, id);
        schedule.update_match_result(&id, &Score::new(1, 2));
        assert!(schedule.tours[0].items.last().unwrap().result.is_some());
    }

    #[test]
    fn test_schedule_tour_new() {
        let schedule_tour = ScheduleTour::new(1, 5);
        assert_eq!(schedule_tour.num, 1);
        assert_eq!(schedule_tour.items.capacity(), 5);
    }

    #[test]
    fn test_schedule_tour_played() {
        let item1 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 15)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: Some(Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }),
        };
        let item2 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 16)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: Some(Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }),
        };
        let items_with_results = vec![item1.clone(), item2.clone()];

        let schedule_tour_with_results = ScheduleTour {
            num: 1,
            items: items_with_results,
        };
        assert!(schedule_tour_with_results.played());

        let item3 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 17)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: None,
        };
        let items_without_results = vec![item1, item3];

        let schedule_tour_without_results = ScheduleTour {
            num: 1,
            items: items_without_results,
        };
        assert!(!schedule_tour_without_results.played());
    }

    #[test]
    fn test_schedule_tour_start_date() {
        let item1 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 15)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: Some(Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }),
        };
        let item2 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 16)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: Some(Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }),
        };
        let schedule_tour = ScheduleTour {
            num: 1,
            items: vec![item1.clone(), item2.clone()],
        };
        assert_eq!(schedule_tour.start_date(), item1.date.date());
    }

    #[test]
    fn test_schedule_tour_end_date() {
        let item1 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 15)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: Some(Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }),
        };
        let item2 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 16)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: Some(Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: Vec::new(),
                home_shootout: 0,
                away_shootout: 0,
            }),
        };
        let schedule_tour = ScheduleTour {
            num: 1,
            items: vec![item1.clone(), item2.clone()],
        };
        assert_eq!(schedule_tour.end_date(), item2.date.date());
    }
}
