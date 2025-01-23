use crate::GameAppData;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use core::{SimulatorData, Team};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct TeamScheduleGetRequest {
    team_slug: String,
}

#[derive(Serialize)]
pub struct TeamScheduleViewModel<'t> {
    pub team_name: &'t str,
    pub team_slug: &'t str,
    pub league_slug: &'t str,
    pub league_name: &'t str,
    pub neighbor_teams: Vec<ClubTeam<'t>>,
    pub items: Vec<TeamScheduleItem<'t>>,
}

#[derive(Serialize)]
pub struct TeamScheduleItem<'t> {
    pub date: String,
    pub time: String,
    pub opponent_slug: &'t str,
    pub opponent_name: &'t str,
    pub is_home: bool,
    pub competition_id: u32,
    pub competition_name: &'t str,
    pub result: Option<TeamScheduleItemResult<'t>>,
}

#[derive(Serialize)]
pub struct TeamScheduleItemResult<'t> {
    pub match_id: &'t str,
    pub home_goals: u8,
    pub away_goals: u8,
}

#[derive(Serialize)]
pub struct ClubTeam<'c> {
    pub slug: &'c str,
    pub name: &'c str,
    pub reputation: u16,
}

pub async fn team_schedule_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamScheduleGetRequest>,
) -> Response {
    let guard = state.data.read().await;

    let simulator_data = guard.as_ref().unwrap();

    let team_id = simulator_data
        .indexes
        .as_ref()
        .unwrap()
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .unwrap();

    let team: &Team = simulator_data.team(team_id).unwrap();

    let league = simulator_data.league(team.league_id).unwrap();

    let schedule = league.schedule.get_matches_for_team(team.id);

    let model = TeamScheduleViewModel {
        team_name: &team.name,
        team_slug: &team.slug,

        league_slug: &league.slug,
        league_name: &league.name,
        neighbor_teams: get_neighbor_teams(team.club_id, simulator_data),

        items: schedule
            .iter()
            .map(|schedule| {
                let is_home = schedule.home_team_id == team.id;

                let home_team_data = simulator_data.team_data(schedule.home_team_id).unwrap();
                let away_team_data = simulator_data.team_data(schedule.away_team_id).unwrap();

                TeamScheduleItem {
                    date: schedule.date.format("%d.%m.%Y").to_string(),
                    time: schedule.date.format("%H:%M").to_string(),
                    opponent_slug: if is_home {
                        &away_team_data.slug
                    } else {
                        &home_team_data.slug
                    },
                    opponent_name: if is_home {
                        &away_team_data.name
                    } else {
                        &home_team_data.name
                    },
                    is_home,
                    competition_id: league.id,
                    competition_name: &league.name,
                    result: if schedule.result.is_some() {
                        Some(TeamScheduleItemResult {
                            match_id: &schedule.id,
                            home_goals: schedule.result.as_ref().unwrap().home_team.get(),
                            away_goals: schedule.result.as_ref().unwrap().away_team.get(),
                        })
                    } else {
                        None
                    },
                }
            })
            .collect(),
    };

    Json(model).into_response()
}

fn get_neighbor_teams(club_id: u32, data: &SimulatorData) -> Vec<ClubTeam> {
    let club = data.club(club_id).unwrap();

    let mut teams: Vec<ClubTeam> = club
        .teams
        .teams
        .iter()
        .map(|team| ClubTeam {
            slug: &team.slug,
            name: &team.name,
            reputation: team.reputation.world,
        })
        .collect();

    teams.sort_by(|a, b| b.reputation.cmp(&a.reputation));

    teams
}
