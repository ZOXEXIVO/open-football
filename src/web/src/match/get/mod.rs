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
    pub lang: String,
    pub league_slug: String,
    pub match_id: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "match/get/index.html")]
pub struct MatchGetTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub league_slug: String,
    pub league_name: String,
    pub match_id: String,
    pub home_team_name: String,
    pub home_team_slug: String,
    pub home_goals: u8,
    pub home_goal_events: Vec<GoalEventDisplay>,
    pub home_squad_main: Vec<MatchPlayer>,
    pub home_squad_subs: Vec<MatchPlayer>,
    pub away_team_name: String,
    pub away_team_slug: String,
    pub away_goals: u8,
    pub away_goal_events: Vec<GoalEventDisplay>,
    pub away_squad_main: Vec<MatchPlayer>,
    pub away_squad_subs: Vec<MatchPlayer>,
    pub match_time_ms: u64,
    pub goals_json: String,
    pub players_json: String,
    pub home_color_primary: String,
    pub home_color_secondary: String,
    pub away_color_primary: String,
    pub away_color_secondary: String,
}

pub struct GoalEventDisplay {
    pub player_id: u32,
    pub player_name: String,
    pub minute: u32,
    pub is_auto_goal: bool,
}

pub struct MatchPlayer {
    pub id: u32,
    pub last_name: String,
    pub position: String,
    pub sub_minute: Option<u32>,
    pub subbed_off_minute: Option<u32>,
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
    let i18n = state.i18n.for_lang(&route_params.lang);
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

    let home_goals = score.home_team.get();
    let away_goals = score.away_team.get();

    let home_goal_events: Vec<GoalEventDisplay> = score
        .detail()
        .iter()
        .filter(|g| {
            let is_home_player = result_details.left_team_players.main.contains(&g.player_id)
                || result_details.left_team_players.substitutes.contains(&g.player_id);
            if g.is_auto_goal { !is_home_player } else { is_home_player }
        })
        .map(|g| {
            let player_name = simulator_data
                .player(g.player_id)
                .map(|p| format!("{} {}", p.full_name.first_name, p.full_name.last_name))
                .unwrap_or_else(|| "Unknown".to_string());
            let minute = if result_details.match_time_ms > 0 {
                (g.time * 90 / result_details.match_time_ms) as u32
            } else { 0 };
            GoalEventDisplay {
                player_id: g.player_id,
                player_name,
                minute,
                is_auto_goal: g.is_auto_goal,
            }
        })
        .collect();

    let away_goal_events: Vec<GoalEventDisplay> = score
        .detail()
        .iter()
        .filter(|g| {
            let is_away_player = result_details.right_team_players.main.contains(&g.player_id)
                || result_details.right_team_players.substitutes.contains(&g.player_id);
            if g.is_auto_goal { !is_away_player } else { is_away_player }
        })
        .map(|g| {
            let player_name = simulator_data
                .player(g.player_id)
                .map(|p| format!("{} {}", p.full_name.first_name, p.full_name.last_name))
                .unwrap_or_else(|| "Unknown".to_string());
            let minute = if result_details.match_time_ms > 0 {
                (g.time * 90 / result_details.match_time_ms) as u32
            } else { 0 };
            GoalEventDisplay {
                player_id: g.player_id,
                player_name,
                minute,
                is_auto_goal: g.is_auto_goal,
            }
        })
        .collect();

    let title = format!(
        "{} - {}",
        home_team.name,
        away_team.name
    );

    Ok(MatchGetTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league.name.clone(),
        sub_title_link: format!("/{}/leagues/{}", &route_params.lang, &league.slug),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections: vec![],
        i18n,
        lang: route_params.lang.clone(),
        league_slug: league.slug.clone(),
        league_name: league.name.clone(),
        match_id: route_params.match_id.clone(),
        home_team_name: home_team.name.clone(),
        home_team_slug: home_team.slug.clone(),
        home_goals,
        home_goal_events,
        home_squad_main: result_details
            .left_team_players
            .main
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data)?;
                if let Some(sub) = result_details.substitutions.iter().find(|s| s.player_out_id == *pid) {
                    p.subbed_off_minute = Some(sub_time_to_minute(sub.match_time_ms, result_details.match_time_ms));
                }
                Some(p)
            })
            .collect(),
        home_squad_subs: result_details
            .left_team_players
            .substitutes
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data)?;
                if let Some(sub) = result_details.substitutions.iter().find(|s| s.player_in_id == *pid) {
                    p.sub_minute = Some(sub_time_to_minute(sub.match_time_ms, result_details.match_time_ms));
                }
                // Check if this sub was also later subbed off (sub-of-sub)
                if let Some(sub_off) = result_details.substitutions.iter().find(|s| s.player_out_id == *pid) {
                    p.subbed_off_minute = Some(sub_time_to_minute(sub_off.match_time_ms, result_details.match_time_ms));
                }
                Some(p)
            })
            .collect(),
        away_team_name: away_team.name.clone(),
        away_team_slug: away_team.slug.clone(),
        away_goals,
        away_goal_events,
        away_squad_main: result_details
            .right_team_players
            .main
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data)?;
                if let Some(sub) = result_details.substitutions.iter().find(|s| s.player_out_id == *pid) {
                    p.subbed_off_minute = Some(sub_time_to_minute(sub.match_time_ms, result_details.match_time_ms));
                }
                Some(p)
            })
            .collect(),
        away_squad_subs: result_details
            .right_team_players
            .substitutes
            .iter()
            .filter_map(|pid| {
                let mut p = to_match_player(*pid, simulator_data)?;
                if let Some(sub) = result_details.substitutions.iter().find(|s| s.player_in_id == *pid) {
                    p.sub_minute = Some(sub_time_to_minute(sub.match_time_ms, result_details.match_time_ms));
                }
                // Check if this sub was also later subbed off (sub-of-sub)
                if let Some(sub_off) = result_details.substitutions.iter().find(|s| s.player_out_id == *pid) {
                    p.subbed_off_minute = Some(sub_time_to_minute(sub_off.match_time_ms, result_details.match_time_ms));
                }
                Some(p)
            })
            .collect(),
        match_time_ms: result_details.match_time_ms,
        goals_json: serde_json::to_string(&goals_json).unwrap_or_else(|_| "[]".to_string()),
        players_json: serde_json::to_string(&players_json).unwrap_or_else(|_| "[]".to_string()),
        home_color_primary: simulator_data
            .club(home_team.club_id)
            .map(|c| c.colors.primary.clone())
            .unwrap_or_else(|| "#00307d".to_string()),
        home_color_secondary: simulator_data
            .club(home_team.club_id)
            .map(|c| c.colors.secondary.clone())
            .unwrap_or_else(|| "#ffffff".to_string()),
        away_color_primary: simulator_data
            .club(away_team.club_id)
            .map(|c| c.colors.primary.clone())
            .unwrap_or_else(|| "#b33f00".to_string()),
        away_color_secondary: simulator_data
            .club(away_team.club_id)
            .map(|c| c.colors.secondary.clone())
            .unwrap_or_else(|| "#ffffff".to_string()),
    })
}

fn to_match_player(player_id: u32, simulator_data: &SimulatorData) -> Option<MatchPlayer> {
    let player = simulator_data.player(player_id)?;
    Some(MatchPlayer {
        id: player.id,
        last_name: player.full_name.last_name.clone(),
        position: player.position().get_short_name().to_string(),
        sub_minute: None,
        subbed_off_minute: None,
    })
}

fn sub_time_to_minute(match_time_ms: u64, total_match_time_ms: u64) -> u32 {
    if total_match_time_ms == 0 {
        return 0;
    }
    (match_time_ms * 90 / total_match_time_ms) as u32
}
