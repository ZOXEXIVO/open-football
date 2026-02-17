pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::league::Season;
use chrono::Datelike;
use core::{Player, SimulatorData, Team};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerHistoryRequest {
    pub team_slug: String,
    pub player_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/history/index.html")]
pub struct PlayerHistoryTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub player_id: u32,
    pub items: Vec<PlayerHistorySeasonItem>,
    pub current_club: String,
    pub current_season: String,
    pub current: PlayerHistoryStats,
}

pub struct PlayerHistorySeasonItem {
    pub season: String,
    pub team_name: String,
    pub is_loan: bool,
    pub stats: PlayerHistoryStats,
}

pub struct PlayerHistoryStats {
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub assists: u16,
    pub penalties: u16,
    pub player_of_the_match: u8,
    pub yellow_cards: u8,
    pub red_cards: u8,
    pub shots_on_target: f32,
    pub passes: u8,
    pub tackling: f32,
    pub average_rating: f32,
}

pub async fn player_history_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerHistoryRequest>,
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

    let team: &Team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let player: &Player = team
        .players
        .players()
        .iter()
        .find(|p| p.id == route_params.player_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Player with ID {} not found in team",
                route_params.player_id
            ))
        })?;

    let neighbor_teams: Vec<(&str, &str)> = get_neighbor_teams(team.club_id, simulator_data)?;

    let mut items: Vec<PlayerHistorySeasonItem> = player
        .statistics_history
        .items
        .iter()
        .map(|item| {
            let season_str = match &item.season {
                Season::OneYear(y) => format!("{}", y),
                Season::TwoYear(y1, y2) => format!("{}/{}", y1, y2 % 100),
            };

            PlayerHistorySeasonItem {
                season: season_str,
                team_name: item.team_name.clone(),
                is_loan: item.is_loan,
                stats: PlayerHistoryStats {
                    played: item.statistics.played,
                    played_subs: item.statistics.played_subs,
                    goals: item.statistics.goals,
                    assists: item.statistics.assists,
                    penalties: item.statistics.penalties,
                    player_of_the_match: item.statistics.player_of_the_match,
                    yellow_cards: item.statistics.yellow_cards,
                    red_cards: item.statistics.red_cards,
                    shots_on_target: item.statistics.shots_on_target,
                    passes: item.statistics.passes,
                    tackling: item.statistics.tackling,
                    average_rating: item.statistics.average_rating,
                },
            }
        })
        .collect();

    // Most recent season first
    items.reverse();

    let current = PlayerHistoryStats {
        played: player.statistics.played,
        played_subs: player.statistics.played_subs,
        goals: player.statistics.goals,
        assists: player.statistics.assists,
        penalties: player.statistics.penalties,
        player_of_the_match: player.statistics.player_of_the_match,
        yellow_cards: player.statistics.yellow_cards,
        red_cards: player.statistics.red_cards,
        shots_on_target: player.statistics.shots_on_target,
        passes: player.statistics.passes,
        tackling: player.statistics.tackling,
        average_rating: player.statistics.average_rating,
    };

    let title = format!("{} {}", player.full_name.first_name, player.full_name.last_name);

    let sim_date = simulator_data.date.date();
    let year = sim_date.year();
    let month = sim_date.month();
    let current_season = if month >= 7 {
        format!("{}/{}", year, (year + 1) % 100)
    } else {
        format!("{}/{}", year - 1, year % 100)
    };

    Ok(PlayerHistoryTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title,
        sub_title: team.name.clone(),
        sub_title_link: format!("/teams/{}", &team.slug),
        menu_sections: views::player_menu(&neighbor_teams, &team.slug),
        team_slug: team.slug.clone(),
        player_id: route_params.player_id,
        items,
        current_club: team.name.clone(),
        current_season,
        current,
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
        .map(|team| {
            (
                team.name.as_str(),
                team.slug.as_str(),
                team.reputation.world,
            )
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}
