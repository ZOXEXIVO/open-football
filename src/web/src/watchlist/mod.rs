pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use core::Player;
use core::PlayerStatusType;
use core::utils::{DateUtils, FormattingUtils};
use serde::Deserialize;
use std::sync::Arc;

pub fn watchlist_routes() -> axum::Router<GameAppData> {
    routes::routes()
}

pub struct WatchlistPlayerDto {
    pub id: u32,
    pub last_name: String,
    pub first_name: String,
    pub position: String,
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
}

#[derive(Deserialize)]
pub struct WatchlistPageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "watchlist/index.html")]
pub struct WatchlistPageTemplate {
    pub css_version: &'static str,
    pub i18n: crate::I18n,
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

    let players: Vec<WatchlistPlayerDto> = simulator_data
        .watchlist
        .iter()
        .filter_map(|&player_id| {
            let (player, team) = simulator_data.player_with_team(player_id)?;
            let country = simulator_data.country(player.country_id)?;
            let position = player.positions.display_positions().join(", ");
            let league = team.league_id.and_then(|id| simulator_data.league(id));

            Some(WatchlistPlayerDto {
                id: player.id,
                first_name: player.full_name.display_first_name().to_string(),
                last_name: player.full_name.display_last_name().to_string(),
                position,
                country_code: country.code.clone(),
                country_name: country.name.clone(),
                country_slug: country.slug.clone(),
                age: DateUtils::age(player.birth_date, now),
                current_ability: get_current_ability_stars(player),
                potential_ability: get_potential_ability_stars(player),
                conditions: get_conditions(player),
                team_name: team.name.clone(),
                team_slug: team.slug.clone(),
                league_name: league.map(|l| l.name.clone()).unwrap_or_default(),
                league_slug: league.map(|l| l.slug.clone()).unwrap_or_default(),
                played: player.statistics.played,
                played_subs: player.statistics.played_subs,
                value: FormattingUtils::format_money(player.value(now)),
                injured: player.player_attributes.is_injured,
                unhappy: !player.happiness.is_happy(),
                transfer_listed: player.statuses.get().contains(&PlayerStatusType::Lst),
            })
        })
        .collect();

    let menu_sections = views::watchlist_menu(&i18n, &route_params.lang, &current_path);

    Ok(WatchlistPageTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
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

    if let Some(ref mut simulator_data) = *guard {
        let player_id = route_params.player_id;
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

    if let Some(ref mut simulator_data) = *guard {
        simulator_data.watchlist.retain(|&id| id != route_params.player_id);
    }

    StatusCode::OK
}

fn get_conditions(player: &Player) -> u8 {
    (100f32 * ((player.player_attributes.condition as f32) / 10000.0)) as u8
}

fn get_current_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.current_ability as f32) / 200.0)) as u8
}

fn get_potential_ability_stars(player: &Player) -> u8 {
    (5.0f32 * ((player.player_attributes.potential_ability as f32) / 200.0)) as u8
}
