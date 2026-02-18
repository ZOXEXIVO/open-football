pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamScheduleGetRequest {
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/schedule/index.html")]
pub struct TeamScheduleTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub league_slug: String,
    pub items: Vec<TeamScheduleItem>,
}

pub struct TeamScheduleItem {
    pub date: String,
    pub time: String,
    pub opponent_slug: String,
    pub opponent_name: String,
    pub is_home: bool,
    pub competition_name: String,
    pub result: Option<TeamScheduleItemResult>,
}

pub struct TeamScheduleItemResult {
    pub match_id: String,
    pub home_goals: u8,
    pub away_goals: u8,
}

pub async fn team_schedule_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamScheduleGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let team_id = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let schedule = league.map(|l| l.schedule.get_matches_for_team(team.id)).unwrap_or_default();

    let neighbor_teams: Vec<(String, String)> = get_neighbor_teams(team.club_id, simulator_data)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let items: Vec<TeamScheduleItem> = schedule
        .iter()
        .map(|schedule| {
            let is_home = schedule.home_team_id == team.id;

            let home_team_data = simulator_data.team_data(schedule.home_team_id).unwrap();
            let away_team_data = simulator_data.team_data(schedule.away_team_id).unwrap();

            TeamScheduleItem {
                date: schedule.date.format("%d.%m.%Y").to_string(),
                time: schedule.date.format("%H:%M").to_string(),
                opponent_slug: if is_home {
                    away_team_data.slug.clone()
                } else {
                    home_team_data.slug.clone()
                },
                opponent_name: if is_home {
                    away_team_data.name.clone()
                } else {
                    home_team_data.name.clone()
                },
                is_home,
                competition_name: league.map(|l| l.name.clone()).unwrap_or_default(),
                result: schedule.result.as_ref().map(|res| TeamScheduleItemResult {
                    match_id: schedule.id.clone(),
                    home_goals: res.home_team.get(),
                    away_goals: res.away_team.get(),
                }),
            }
        })
        .collect();

    Ok(TeamScheduleTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title: team.name.clone(),
        sub_title: league.map(|l| l.name.clone()).unwrap_or_default(),
        sub_title_link: league.map(|l| format!("/leagues/{}", &l.slug)).unwrap_or_default(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.primary.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.secondary.clone()).unwrap_or_default(),
        menu_sections: views::team_menu(&neighbor_refs, &team.slug, &format!("/teams/{}/schedule", &team.slug)),
        team_slug: team.slug.clone(),
        league_slug: league.map(|l| l.slug.clone()).unwrap_or_default(),
        items,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
) -> Result<Vec<(String, String)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| (team.team_type.to_string(), team.slug.clone(), team.reputation.world))
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}
