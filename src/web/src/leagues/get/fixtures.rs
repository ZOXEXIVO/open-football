use super::{LeagueScheduleItem, TourSchedule};
use chrono::Duration;
use core::league::{League, ScheduleTour};
use core::{SimulatorData, Team};
use itertools::Itertools;
use serde::Serialize;

pub(super) struct LeagueFixtures;

impl LeagueFixtures {
    pub(super) fn schedule(data: &SimulatorData, league: &League) -> Vec<TourSchedule> {
        let today = data.date.date();
        let tours = &league.schedule.tours;

        // Keep the last started round visible and unveil the next one on its eve.
        let latest_started = tours.iter().rfind(|t| t.start_date() <= today);
        let reveal_date = today + Duration::days(1);
        let next_up = tours
            .iter()
            .find(|t| t.start_date() > today && t.start_date() <= reveal_date);

        let mut shown_tours: Vec<&ScheduleTour> = Vec::new();
        shown_tours.extend(latest_started);
        shown_tours.extend(next_up);

        // In pre-season, show the opening round even before its reveal date.
        if shown_tours.is_empty() {
            shown_tours.extend(tours.first());
        }

        let mut schedule = Vec::new();
        for tour in shown_tours {
            for (date, group) in &tour.items.iter().chunk_by(|t| t.date.date()) {
                schedule.push(TourSchedule {
                    date: date.format("%d.%m.%Y").to_string(),
                    matches: group
                        .map(|item| LeagueScheduleItem::from_item(data, item))
                        .collect(),
                });
            }
        }
        schedule
    }

    pub(super) fn completed(schedule: &[TourSchedule]) -> Vec<CompletedMatch> {
        schedule
            .iter()
            .flat_map(|day| &day.matches)
            .filter_map(|item| {
                let result = item.result.as_ref()?;
                Some(CompletedMatch {
                    match_id: item.match_id.clone(),
                    home_team: item.home_team_name.clone(),
                    away_team: item.away_team_name.clone(),
                    home_goals: result.home_goals,
                    away_goals: result.away_goals,
                    fan_context: result.fan_context,
                })
            })
            .collect()
    }
}

#[derive(Hash, Serialize)]
pub(super) struct CompletedMatch {
    pub(super) match_id: String,
    pub(super) home_team: String,
    pub(super) away_team: String,
    pub(super) home_goals: u8,
    pub(super) away_goals: u8,
    #[serde(flatten)]
    fan_context: MatchFanContext,
}

#[derive(Clone, Copy, Hash, Serialize)]
pub(super) struct MatchFanContext {
    club_rivalry: bool,
    home_national_reputation: u16,
    away_national_reputation: u16,
}

impl MatchFanContext {
    pub(super) fn for_teams(data: &SimulatorData, home: &Team, away: &Team) -> Self {
        let club_rivalry = data
            .club(home.club_id)
            .is_some_and(|club| club.is_rival(away.club_id))
            || data
                .club(away.club_id)
                .is_some_and(|club| club.is_rival(home.club_id));
        Self {
            club_rivalry,
            home_national_reputation: home.reputation.national,
            away_national_reputation: away.reputation.national,
        }
    }
}
