pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamStatsRequest {
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/stats/index.html")]
pub struct TeamStatsTemplate {
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub players: Vec<TeamPlayerStats>,
}

pub struct TeamPlayerStats {
    pub id: u32,
    pub last_name: String,
    pub first_name: String,
    pub position: String,
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub assists: u16,
    pub yellow_cards: u8,
    pub red_cards: u8,
    pub shots_on_target: f32,
    pub passes: u8,
    pub tackling: f32,
    pub average_rating: f32,
}

pub async fn team_stats_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamStatsRequest>,
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

    let mut players: Vec<TeamPlayerStats> = team
        .players()
        .iter()
        .map(|p| TeamPlayerStats {
            id: p.id,
            last_name: p.full_name.last_name.clone(),
            first_name: p.full_name.first_name.clone(),
            position: p.position().get_short_name().to_string(),
            played: p.statistics.played,
            played_subs: p.statistics.played_subs,
            goals: p.statistics.goals,
            assists: p.statistics.assists,
            yellow_cards: p.statistics.yellow_cards,
            red_cards: p.statistics.red_cards,
            shots_on_target: p.statistics.shots_on_target,
            passes: p.statistics.passes,
            tackling: p.statistics.tackling,
            average_rating: p.statistics.average_rating,
        })
        .collect();

    players.sort_by(|a, b| b.average_rating.partial_cmp(&a.average_rating).unwrap_or(std::cmp::Ordering::Equal));

    Ok(TeamStatsTemplate {
        title: team.name.clone(),
        sub_title: league.name.clone(),
        sub_title_link: format!("/leagues/{}", &league.slug),
        menu_sections: views::team_menu(&neighbor_teams, &team.slug),
        team_slug: team.slug.clone(),
        players,
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
