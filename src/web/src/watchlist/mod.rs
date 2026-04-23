pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use core::utils::{DateUtils, FormattingUtils};
use chrono::NaiveDate;
use core::Player;
use core::PlayerPositionType;
use core::PlayerStatusType;
use core::SimulatorData;
use serde::Deserialize;
use std::sync::Arc;

pub fn watchlist_routes() -> axum::Router<GameAppData> {
    routes::routes()
}

pub struct WatchlistPlayerDto {
    pub id: u32,
    pub slug: String,
    pub last_name: String,
    pub first_name: String,
    pub position: String,
    pub position_sort: PlayerPositionType,
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub age: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub conditions: u8,
    pub team_name: String,
    pub team_slug: String,
    pub league_name: String,
    pub league_slug: String,
    pub played: u16,
    pub played_subs: u16,
    pub value: String,
    pub injured: bool,
    pub unhappy: bool,
    pub transfer_listed: bool,
    pub retired: bool,
}

#[derive(Deserialize)]
pub struct WatchlistPageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "watchlist/index.html")]
pub struct WatchlistPageTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub players: Vec<WatchlistPlayerDto>,
}

pub async fn watchlist_page_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<WatchlistPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let now = simulator_data.date.date();
    let current_path = format!("/{}/watchlist", &route_params.lang);

    let mut players: Vec<WatchlistPlayerDto> = simulator_data
        .watchlist
        .iter()
        .filter_map(|&player_id| {
            if let Some((player, team)) = simulator_data.player_with_team(player_id) {
                let league = team.league_id.and_then(|id| simulator_data.league(id));
                let head_coach = team.staffs.head_coach();
                Some(WatchlistPlayerDto {
                    potential_ability: get_potential_ability_stars_by_staff(
                        player,
                        head_coach.staff_attributes.knowledge.judging_player_potential,
                        head_coach.id,
                    ),
                    team_name: team.name.clone(),
                    team_slug: team.slug.clone(),
                    league_name: league.map(|l| l.name.clone()).unwrap_or_default(),
                    league_slug: league.map(|l| l.slug.clone()).unwrap_or_default(),
                    value: FormattingUtils::format_money(player.value(
                        now,
                        league.map(|l| l.reputation).unwrap_or(0),
                        team.reputation.world,
                    )),
                    unhappy: !player.happiness.is_happy(),
                    transfer_listed: player.statuses.get().contains(&PlayerStatusType::Lst),
                    ..base_watchlist_dto(player, simulator_data, now)
                })
            } else if let Some(player) = simulator_data.retired_player(player_id) {
                Some(WatchlistPlayerDto {
                    conditions: 0,
                    team_name: i18n.t("retired").to_string(),
                    retired: true,
                    ..base_watchlist_dto(player, simulator_data, now)
                })
            } else if let Some(player) = simulator_data.free_agents.iter().find(|p| p.id == player_id) {
                // Player was released to the global free-agent pool (via the
                // "move to free agent" action) — no team, not retired. Without
                // this branch the watchlist silently drops him on next render.
                Some(WatchlistPlayerDto {
                    team_name: i18n.t("free_agent").to_string(),
                    ..base_watchlist_dto(player, simulator_data, now)
                })
            } else {
                None
            }
        })
        .collect();

    players.sort_by(|a, b| {
        a.position_sort
            .partial_cmp(&b.position_sort)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let menu_sections = views::watchlist_menu(&i18n, &route_params.lang, &current_path);

    Ok(WatchlistPageTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        i18n,
        lang: route_params.lang.clone(),
        title: "Watch List".to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: String::new(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections,
        players,
    })
}

#[derive(Deserialize)]
pub struct WatchlistModifyRequest {
    pub player_id: u32,
}

pub async fn watchlist_add_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<WatchlistModifyRequest>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let player_id = route_params.player_id;
        let simulator_data = Arc::make_mut(arc_data);
        if !simulator_data.watchlist.contains(&player_id) {
            simulator_data.watchlist.push(player_id);
        }
    }

    StatusCode::OK
}

pub async fn watchlist_remove_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<WatchlistModifyRequest>,
) -> impl IntoResponse {
    let data = Arc::clone(&state.data);
    let mut guard = data.write().await;

    if let Some(ref mut arc_data) = *guard {
        let simulator_data = Arc::make_mut(arc_data);
        simulator_data.watchlist.retain(|&id| id != route_params.player_id);
    }

    StatusCode::OK
}

/// DTO filled from fields that don't depend on the player's current team.
/// Caller overrides team/league/value/potential/etc via struct-update syntax.
fn base_watchlist_dto(
    player: &Player,
    simulator_data: &SimulatorData,
    now: NaiveDate,
) -> WatchlistPlayerDto {
    let (country_code, country_name, country_slug) = simulator_data
        .country(player.country_id)
        .map(|c| (c.code.clone(), c.name.clone(), c.slug.clone()))
        .or_else(|| {
            simulator_data
                .country_info
                .get(&player.country_id)
                .map(|i| (i.code.clone(), i.name.clone(), i.slug.clone()))
        })
        .unwrap_or_default();

    WatchlistPlayerDto {
        id: player.id,
        slug: player.slug(),
        first_name: player.full_name.display_first_name().to_string(),
        last_name: player.full_name.display_last_name().to_string(),
        position: player.positions.display_positions_compact(),
        position_sort: player.position(),
        country_code,
        country_name,
        country_slug,
        age: DateUtils::age(player.birth_date, now),
        current_ability: get_current_ability_stars(player),
        potential_ability: get_potential_ability_stars(player),
        conditions: get_conditions(player),
        team_name: String::new(),
        team_slug: String::new(),
        league_name: String::new(),
        league_slug: String::new(),
        played: player.statistics.played,
        played_subs: player.statistics.played_subs,
        value: "-".to_string(),
        injured: player.player_attributes.is_injured,
        unhappy: false,
        transfer_listed: false,
        retired: false,
    }
}

fn get_conditions(player: &Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}

fn get_current_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.current_ability as f32) / 200.0)).round() as u8
}

fn get_potential_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.potential_ability as f32) / 200.0)).round() as u8
}

fn get_potential_ability_stars_by_staff(player: &Player, staff_judging: u8, staff_id: u32) -> u8 {
    let raw_stars = 5.0 * (player.player_attributes.potential_ability as f32 / 200.0);
    let accuracy = (staff_judging as f32 / 20.0).clamp(0.0, 1.0);
    let noise_scale = (1.0 - accuracy) * 1.5;

    let hash = staff_id
        .wrapping_mul(2654435761)
        .wrapping_add(player.id.wrapping_mul(2246822519));
    let hash = hash ^ (hash >> 16);
    let hash = hash.wrapping_mul(0x45d9f3b);
    let hash = hash ^ (hash >> 16);
    let noise = (hash & 0xFFFF) as f32 / 32768.0 - 1.0;

    (raw_stars + noise * noise_scale).round().clamp(0.0, 5.0) as u8
}
