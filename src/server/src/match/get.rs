use crate::{ApiError, ApiResult, GameAppData};
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use core::SimulatorData;
use serde::{Deserialize, Serialize};

pub async fn match_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<MatchGetRequest>,
) -> ApiResult<Response> {
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
        .ok_or_else(|| ApiError::NotFound(format!("League '{}' not found", route_params.league_slug)))?;

    let league = simulator_data
        .league(league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", league_id)))?;

    let match_result = league
        .matches
        .get(&route_params.match_id)
        .ok_or_else(|| ApiError::NotFound(format!("Match '{}' not found", route_params.match_id)))?;

    let home_team = simulator_data
        .team(match_result.home_team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Home team not found")))?;

    let away_team = simulator_data
        .team(match_result.away_team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Away team not found")))?;

    let result_details = match_result
        .details
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Match details not available".to_string()))?;

    let score = result_details
        .score
        .as_ref()
        .ok_or_else(|| ApiError::NotFound("Match score not available".to_string()))?;

    let goals: Vec<GoalEvent> = score
        .detail()
        .iter()
        .map(|goal| GoalEvent {
            player_id: goal.player_id,
            time: goal.time,
            is_auto_goal: goal.is_auto_goal,
        })
        .collect();

    let result = MatchGetResponse {
        score: MatchScore {
            home_goals: score.home_team.get(),
            away_goals: score.away_team.get()
        },
        match_time_ms: result_details.match_time_ms,
        goals,
        home_team_name: &home_team.name,
        home_team_slug: &home_team.slug,
        home_squad: MatchSquad {
            main: result_details
                .left_team_players
                .main
                .iter()
                .filter_map(|player_id| to_match_player(*player_id, simulator_data))
                .collect(),
            substitutes: result_details
                .left_team_players
                .substitutes
                .iter()
                .filter_map(|player_id| to_match_player(*player_id, simulator_data))
                .collect(),
        },
        away_team_name: &away_team.name,
        away_team_slug: &away_team.slug,
        away_squad: MatchSquad {
            main: result_details
                .right_team_players
                .main
                .iter()
                .filter_map(|player_id| to_match_player(*player_id, simulator_data))
                .collect(),
            substitutes: result_details
                .right_team_players
                .substitutes
                .iter()
                .filter_map(|player_id| to_match_player(*player_id, simulator_data))
                .collect(),
        },
    };

    Ok(Json(result).into_response())
}

fn to_match_player(
    player_id: u32,
    simulator_data: &SimulatorData,
) -> Option<MatchPlayer<'_>> {
    let player = simulator_data.player(player_id)?;

    Some(MatchPlayer {
        id: player.id,
        shirt_number: player.shirt_number(),
        first_name: &player.full_name.first_name,
        last_name: &player.full_name.last_name,
        middle_name: player.full_name.middle_name.as_deref(),
        position: player.position().get_short_name(),
    })
}


#[derive(Deserialize)]
pub struct MatchGetRequest {
    pub league_slug: String,
    pub match_id: String,
}

#[derive(Serialize)]
pub struct MatchGetResponse<'p> {
    // home
    pub home_team_name: &'p str,
    pub home_team_slug: &'p str,
    pub home_squad: MatchSquad<'p>,

    // away
    pub away_team_name: &'p str,
    pub away_team_slug: &'p str,
    pub away_squad: MatchSquad<'p>,

    pub match_time_ms: u64,

    pub score: MatchScore,

    pub goals: Vec<GoalEvent>,
}

#[derive(Serialize)]
pub struct MatchScore {
    pub home_goals: u8,
    pub away_goals: u8,
}

#[derive(Serialize)]
pub struct GoalEvent {
    pub player_id: u32,
    pub time: u64,
    pub is_auto_goal: bool,
}

#[derive(Serialize)]
pub struct MatchSquad<'p> {
    pub main: Vec<MatchPlayer<'p>>,
    pub substitutes: Vec<MatchPlayer<'p>>,
}

#[derive(Serialize)]
pub struct MatchPlayer<'p> {
    pub id: u32,
    pub shirt_number: u8,
    pub first_name: &'p str,
    pub last_name: &'p str,
    pub middle_name: Option<&'p str>,
    pub position: &'p str,
}
