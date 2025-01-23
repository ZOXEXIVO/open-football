use crate::GameAppData;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Duration;
use core::league::ScheduleTour;
use itertools::*;
use serde::{Deserialize, Serialize};
use core::r#match::Score;
use core::r#match::statistics::MatchStatisticType;
use core::r#match::GoalDetail;

#[derive(Deserialize)]
pub struct LeagueGetRequest {
    pub league_slug: String,
}

#[derive(Serialize)]
pub struct LeagueGetViewModel<'l> {
    pub id: u32,
    pub name: &'l str,
    pub slug: &'l str,
    pub country_slug: &'l str,
    pub country_name: &'l str,
    pub table: LeagueTableDto<'l>,
    pub current_tour_schedule: Vec<TourSchedule<'l>>,
}

#[derive(Serialize)]
pub struct TourSchedule<'s> {
    pub date: String,
    pub matches: Vec<LeagueScheduleItem<'s>>,
}

#[derive(Serialize)]
pub struct LeagueScheduleItem<'si> {
    pub match_id: &'si str,

    pub home_team_id: u32,
    pub home_team_name: &'si str,
    pub home_team_slug: &'si str,

    pub away_team_id: u32,
    pub away_team_name: &'si str,
    pub away_team_slug: &'si str,

    pub result: Option<LeagueScheduleItemResult>,
}

#[derive(Serialize)]
pub struct LeagueScheduleItemResult {
    pub home_goals: u8,
    pub home_goalscorers: Vec<LeagueTableGoalscorer>,

    pub away_goals: u8,
    pub away_goalscorers: Vec<LeagueTableGoalscorer>,
}

impl From<&Score> for LeagueScheduleItemResult {
    fn from(value: &Score) -> Self {
        todo!()
    }
}

#[derive(Serialize)]
pub struct LeagueTableGoalscorer {
    pub id: u32,
    pub name: String,
}

#[derive(Serialize)]
pub struct LeagueTableDto<'l> {
    pub rows: Vec<LeagueTableRow<'l>>,
}

#[derive(Serialize)]
pub struct LeagueTableRow<'l> {
    pub team_id: u32,
    pub team_name: &'l str,
    pub team_slug: &'l str,
    pub played: u8,
    pub win: u8,
    pub draft: u8,
    pub lost: u8,
    pub goal_scored: i32,
    pub goal_concerned: i32,
    pub points: u8,
}

pub async fn league_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<LeagueGetRequest>,
) -> Response {
    let guard = state.data.read().await;

    let simulator_data = guard.as_ref().unwrap();

    let league_id = simulator_data
        .indexes
        .as_ref()
        .unwrap()
        .slug_indexes
        .get_league_by_slug(&route_params.league_slug)
        .unwrap();

    let league = simulator_data.league(league_id).unwrap();

    let country = simulator_data.country(league.country_id).unwrap();

    let league_table = league.table.get();

    let mut model = LeagueGetViewModel {
        id: league.id,
        name: &league.name,
        slug: &league.slug,
        country_slug: &country.slug,
        country_name: &country.name,
        table: LeagueTableDto {
            rows: league_table
                .iter()
                .map(|t| {
                    let team_data = simulator_data.team_data(t.team_id).unwrap();
                    LeagueTableRow {
                        team_id: t.team_id,
                        team_name: &team_data.name,
                        team_slug: &team_data.slug,
                        played: t.played,
                        win: t.win,
                        draft: t.draft,
                        lost: t.lost,
                        goal_scored: t.goal_scored,
                        goal_concerned: t.goal_concerned,
                        points: t.points,
                    }
                })
                .collect(),
        },
        current_tour_schedule: Vec::new(),
    };

    let now = simulator_data.date.date() + Duration::days(3);

    let mut current_tour: Option<&ScheduleTour> = None;

    for tour in league.schedule.tours.iter() {
        if now >= tour.start_date() && now <= tour.end_date() {
            current_tour = Some(tour);
        }
    }

    if current_tour.is_none() {
        for tour in league.schedule.tours.iter() {
            if now >= tour.end_date() {
                current_tour = Some(tour);
            }
        }
    }

    if current_tour.is_some() {
        for (key, group) in &current_tour
            .as_ref()
            .unwrap()
            .items
            .iter()
            .chunk_by(|t| t.date.date())
        {
            let tour_schedule = TourSchedule {
                date: key.format("%d.%m.%Y").to_string(),
                matches: group
                    .map(|item| {
                        let home_team_data = simulator_data.team_data(item.home_team_id).unwrap();
                        let home_team = simulator_data.team(item.home_team_id).unwrap();

                        let away_team_data = simulator_data.team_data(item.away_team_id).unwrap();
                        let away_team = simulator_data.team(item.away_team_id).unwrap();

                        LeagueScheduleItem {
                            match_id: &item.id,

                            result: item.result.as_ref().map(|res| {
                                let details: Vec<&GoalDetail> = res.details.iter()
                                    .filter(|detail| detail.stat_type == MatchStatisticType::Goal)
                                    .collect();

                                return LeagueScheduleItemResult {
                                    home_goals: if item.home_team_id == res.home_team.team_id {
                                        res.home_team.get()
                                    } else {
                                        res.away_team.get()
                                    },
                                    home_goalscorers: details.iter().filter_map(|detail| {
                                        let player = simulator_data.player(detail.player_id).unwrap();
                                        if home_team.players.contains(player.id) {
                                            Some(LeagueTableGoalscorer {
                                                id: detail.player_id,
                                                name: player.full_name.to_string(),
                                            })
                                        } else {
                                            None
                                        }
                                    }).collect(),
                                    away_goals: if item.away_team_id == res.away_team.team_id {
                                        res.away_team.get()
                                    } else {
                                        res.home_team.get()
                                    },
                                    away_goalscorers: details.iter().filter_map(|detail| {
                                        let player = simulator_data.player(detail.player_id).unwrap();
                                        if away_team.players.contains(player.id) {
                                            Some(LeagueTableGoalscorer {
                                                id: detail.player_id,
                                                name: player.full_name.to_string(),
                                            })
                                        } else {
                                            None
                                        }
                                    }).collect(),
                                }
                            }),

                            home_team_id: item.home_team_id,
                            home_team_name: &simulator_data
                                .team_data(item.home_team_id)
                                .unwrap()
                                .name,
                            home_team_slug: &home_team_data.slug,

                            away_team_id: item.away_team_id,
                            away_team_name: &simulator_data
                                .team_data(item.away_team_id)
                                .unwrap()
                                .name,
                            away_team_slug: &away_team_data.slug,
                        }
                    })
                    .collect(),
            };

            model.current_tour_schedule.push(tour_schedule)
        }
    }

    Json(model).into_response()
}
