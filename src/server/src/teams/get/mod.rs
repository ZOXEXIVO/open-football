pub mod routes;

use crate::player::PlayerStatusDto;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::Player;
use core::PlayerPositionType;
use core::utils::FormattingUtils;
use core::{SimulatorData, Team};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamGetRequest {
    pub team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/get/index.html")]
pub struct TeamGetTemplate {
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub players: Vec<TeamPlayer>,
}

pub struct TeamPlayer {
    pub id: u32,
    pub last_name: String,
    pub first_name: String,
    pub behaviour: String,
    pub position: String,
    pub position_sort: PlayerPositionType,
    pub value: String,
    pub injured: bool,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub conditions: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub status: PlayerStatusDto,
}

pub async fn team_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamGetRequest>,
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
        .ok_or_else(|| ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug)))?;

    let team: &Team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = simulator_data
        .league(team.league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", team.league_id)))?;

    let now = simulator_data.date.date();

    let mut players: Vec<TeamPlayer> = team
        .players()
        .iter()
        .filter_map(|p| {
            let country = simulator_data.country(p.country_id)?;
            let position = p.positions.display_positions().join(", ");

            Some(TeamPlayer {
                id: p.id,
                first_name: p.full_name.first_name.clone(),
                position_sort: p.position(),
                position,
                behaviour: p.behaviour.as_str().to_string(),
                injured: p.player_attributes.is_injured,
                country_slug: country.slug.clone(),
                country_code: country.code.clone(),
                country_name: country.name.clone(),
                last_name: p.full_name.last_name.clone(),
                conditions: get_conditions(p),
                value: FormattingUtils::format_money(p.value(now)),
                current_ability: get_current_ability_stars(p),
                potential_ability: get_potential_ability_stars(p),
                status: PlayerStatusDto::new(p.statuses.get()),
            })
        })
        .collect();

    players.sort_by(|a, b| {
        a.position_sort
            .partial_cmp(&b.position_sort)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let neighbor_teams: Vec<(&str, &str)> = get_neighbor_teams(team.club_id, simulator_data)?;

    Ok(TeamGetTemplate {
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

pub fn get_conditions(player: &Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}

pub fn get_current_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.current_ability as f32) / 200.0)) as u8
}

pub fn get_potential_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.potential_ability as f32) / 200.0)) as u8
}
