use crate::context::GlobalContext;
use crate::league::round::RoundSchedule;
use crate::league::{LeagueMatch, LeagueSettings, ScheduleGenerator, ScheduleResult, Season};
use crate::r#match::Score;
use chrono::{Datelike, NaiveDate, NaiveDateTime};
use log::error;

#[derive(Debug, Default)]
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
        ctx: GlobalContext<'_>,
    ) -> ScheduleResult {
        let mut result = ScheduleResult::new();

        if self.tours.is_empty() || league_settings.is_time_for_new_schedule(&ctx.simulation) {
            let league_ctx = ctx.league.as_ref().unwrap();

            let generator = RoundSchedule::new();

            match generator.generate(
                league_ctx.id,
                &league_ctx.slug,
                Season::OneYear(ctx.simulation.date.year() as u16),
                league_ctx.team_ids,
                league_settings,
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
            })
            .collect();

        result
    }

    pub fn get_matches(&self, date: NaiveDateTime) -> Vec<ScheduleItem> {
        self.tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(|s| s.date == date)
            .map(|s| {
                ScheduleItem::new(
                    s.league_id,
                    String::from(&s.league_slug),
                    s.home_team_id,
                    s.away_team_id,
                    s.date,
                    None,
                )
            })
            .collect()
    }

    pub fn get_matches_for_team(&self, team_id: u32) -> Vec<ScheduleItem> {
        self.tours
            .iter()
            .flat_map(|t| &t.items)
            .filter(|s| s.home_team_id == team_id || s.away_team_id == team_id)
            .map(|s| {
                ScheduleItem::new(
                    s.league_id,
                    String::from(&s.league_slug),
                    s.home_team_id,
                    s.away_team_id,
                    s.date,
                    s.result.clone(),
                )
            })
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
}


#[derive(Debug, Clone)]
pub struct ScheduleError {
    pub message: String,
}

impl ScheduleError {
    pub fn from_str(str: &'static str) -> Self {
        ScheduleError {
            message: str.to_owned(),
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
            league_slug: String::from(league_slug),
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
            }),
        };
        let mut items_with_results = Vec::new();
        items_with_results.push(item1.clone());
        items_with_results.push(item2.clone());

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
        let mut items_without_results = Vec::new();
        items_without_results.push(item1);
        items_without_results.push(item3);

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
            }),
        };
        let item2 = ScheduleItem {
            id: "".to_string(),
            league_id: 0,
            league_slug: "slug".to_string(),
            date: NaiveDate::from_ymd_opt(2024, 3, 16).unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap(),
            home_team_id: 0,
            away_team_id: 0,
            result: Some(Score {
                home_team: TeamScore::new_with_score(0, 0),
                away_team: TeamScore::new_with_score(0, 0),
                details: Vec::new(),
            }),
        };
        let schedule_tour = ScheduleTour {
            num: 1,
            items: vec![item1.clone(), item2.clone()],
        };
        assert_eq!(schedule_tour.end_date(), item2.date.date());
    }
}
