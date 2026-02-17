pub mod routes;

use crate::views::MenuSection;
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct MatchGetRequest {
    pub league_slug: String,
    pub match_id: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "match/get/index.html")]
pub struct MatchGetTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub menu_sections: Vec<MenuSection>,
    pub league_slug: String,
    pub match_id: String,
    pub home_team_name: String,
    pub home_team_slug: String,
    pub home_squad_main: Vec<MatchPlayer>,
    pub home_squad_subs: Vec<MatchPlayer>,
    pub away_team_name: String,
    pub away_team_slug: String,
    pub away_squad_main: Vec<MatchPlayer>,
    pub away_squad_subs: Vec<MatchPlayer>,
    pub match_time_ms: u64,
    pub goals_json: String,
    pub players_json: String,
}

pub struct MatchPlayer {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    pub position: String,
}

#[derive(Serialize)]
struct GoalEventJson {
    player_id: u32,
    time: u64,
    is_auto_goal: bool,
}

#[derive(Serialize)]
struct PlayerJson {
    id: u32,
    shirt_number: u8,
    last_name: String,
    position: String,
    is_home: bool,
}

pub async fn match_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<MatchGetRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let league_id = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?
        .slug_indexes
        .get_league_by_slug(&route_params.league_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("League '{}' not found", route_params.league_slug))
        })?;

    let league = simulator_data
        .league(league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", league_id)))?;

    let match_result = league
        .matches
        .get(&route_params.match_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Match '{}' not found", route_params.match_id))
        })?;

    let home_team = simulator_data
        .team(match_result.home_team_id)
        .ok_or_else(|| ApiError::NotFound("Home team not found".to_string()))?;

    let away_team = simulator_data
        .team(match_result.away_team_id)
        .ok_or_else(|| ApiError::NotFound("Away team not found".to_string()))?;

    let result_details = match_result
        .details
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Match details not available".to_string()))?;

    let score = result_details
        .score
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Match score not available".to_string()))?;

    let goals_json: Vec<GoalEventJson> = score
        .detail()
        .iter()
        .map(|goal| GoalEventJson {
            player_id: goal.player_id,
            time: goal.time,
            is_auto_goal: goal.is_auto_goal,
        })
        .collect();

    let mut players_json: Vec<PlayerJson> = Vec::new();

    // Assign squad numbers (1-based) per team when shirt_number is not set
    let mut home_number: u8 = 1;
    for player_id in &result_details.left_team_players.main {
        if let Some(p) = simulator_data.player(*player_id) {
            let sn = p.shirt_number();
            let number = if sn == 0 { home_number } else { sn };
            players_json.push(PlayerJson {
                id: p.id,
                shirt_number: number,
                last_name: p.full_name.last_name.clone(),
                position: p.position().get_short_name().to_string(),
                is_home: true,
            });
            home_number += 1;
        }
    }
    for player_id in &result_details.left_team_players.substitutes {
        if let Some(p) = simulator_data.player(*player_id) {
            let sn = p.shirt_number();
            let number = if sn == 0 { home_number } else { sn };
            players_json.push(PlayerJson {
                id: p.id,
                shirt_number: number,
                last_name: p.full_name.last_name.clone(),
                position: p.position().get_short_name().to_string(),
                is_home: true,
            });
            home_number += 1;
        }
    }

    let mut away_number: u8 = 1;
    for player_id in &result_details.right_team_players.main {
        if let Some(p) = simulator_data.player(*player_id) {
            let sn = p.shirt_number();
            let number = if sn == 0 { away_number } else { sn };
            players_json.push(PlayerJson {
                id: p.id,
                shirt_number: number,
                last_name: p.full_name.last_name.clone(),
                position: p.position().get_short_name().to_string(),
                is_home: false,
            });
            away_number += 1;
        }
    }
    for player_id in &result_details.right_team_players.substitutes {
        if let Some(p) = simulator_data.player(*player_id) {
            let sn = p.shirt_number();
            let number = if sn == 0 { away_number } else { sn };
            players_json.push(PlayerJson {
                id: p.id,
                shirt_number: number,
                last_name: p.full_name.last_name.clone(),
                position: p.position().get_short_name().to_string(),
                is_home: false,
            });
            away_number += 1;
        }
    }

    let title = format!(
        "{} {} - {} {}",
        home_team.name,
        score.home_team.get(),
        score.away_team.get(),
        away_team.name
    );

    Ok(MatchGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title,
        sub_title: league.name.clone(),
        sub_title_link: format!("/leagues/{}", &league.slug),
        menu_sections: vec![],
        league_slug: league.slug.clone(),
        match_id: route_params.match_id.clone(),
        home_team_name: home_team.name.clone(),
        home_team_slug: home_team.slug.clone(),
        home_squad_main: result_details
            .left_team_players
            .main
            .iter()
            .filter_map(|pid| to_match_player(*pid, simulator_data))
            .collect(),
        home_squad_subs: result_details
            .left_team_players
            .substitutes
            .iter()
            .filter_map(|pid| to_match_player(*pid, simulator_data))
            .collect(),
        away_team_name: away_team.name.clone(),
        away_team_slug: away_team.slug.clone(),
        away_squad_main: result_details
            .right_team_players
            .main
            .iter()
            .filter_map(|pid| to_match_player(*pid, simulator_data))
            .collect(),
        away_squad_subs: result_details
            .right_team_players
            .substitutes
            .iter()
            .filter_map(|pid| to_match_player(*pid, simulator_data))
            .collect(),
        match_time_ms: result_details.match_time_ms,
        goals_json: serde_json::to_string(&goals_json).unwrap_or_else(|_| "[]".to_string()),
        players_json: serde_json::to_string(&players_json).unwrap_or_else(|_| "[]".to_string()),
    })
}

fn to_match_player(player_id: u32, simulator_data: &SimulatorData) -> Option<MatchPlayer> {
    let player = simulator_data.player(player_id)?;
    Some(MatchPlayer {
        id: player.id,
        first_name: player.full_name.first_name.clone(),
        last_name: player.full_name.last_name.clone(),
        position: player.position().get_short_name().to_string(),
    })
}
