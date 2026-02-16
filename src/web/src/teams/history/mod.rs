pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamHistoryRequest {
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/history/index.html")]
pub struct TeamHistoryTemplate {
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub items: Vec<TeamHistoryMatchItem>,
}

pub struct TeamHistoryMatchItem {
    pub date: String,
    pub opponent_name: String,
    pub home_goals: u8,
    pub away_goals: u8,
}

pub async fn team_history_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamHistoryRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let team_id = indexes
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = simulator_data
        .league(team.league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", team.league_id)))?;

    let neighbor_teams: Vec<(&str, &str)> = get_neighbor_teams(team.club_id, simulator_data)?;

    let items: Vec<TeamHistoryMatchItem> = team
        .match_history
        .items()
        .iter()
        .map(|item| {
            let opponent_name = simulator_data
                .team_data(item.rival_team_id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            TeamHistoryMatchItem {
                date: item.date.format("%d.%m.%Y").to_string(),
                opponent_name,
                home_goals: item.score.0.get(),
                away_goals: item.score.1.get(),
            }
        })
        .collect();

    Ok(TeamHistoryTemplate {
        title: team.name.clone(),
        sub_title: league.name.clone(),
        sub_title_link: format!("/leagues/{}", &league.slug),
        menu_sections: views::team_menu(&neighbor_teams, &team.slug),
        team_slug: team.slug.clone(),
        items,
    })
}

fn get_neighbor_teams<'a>(
    club_id: u32,
    data: &'a SimulatorData,
) -> Result<Vec<(&'a str, &'a str)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(&str, &str, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| (team.name.as_str(), team.slug.as_str(), team.reputation.world))
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams.into_iter().map(|(name, slug, _)| (name, slug)).collect())
}
