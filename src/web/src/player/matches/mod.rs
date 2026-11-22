pub mod collector;
pub mod routes;

use crate::player::matches::collector::PlayerMatchCollector;
use crate::player::{PlayerPageChrome, PlayerPageColors};
use crate::shared::season_step::SeasonStep;
use crate::shared::slug::{PlayerPage, PlayerSlug};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use core::league::season::LeagueSeason;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerMatchesRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Deserialize)]
pub struct PlayerMatchesQuery {
    pub season: Option<i32>,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/matches/index.html")]
pub struct PlayerMatchesTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_flag: Option<SubTitleFlag>,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub player_id: u32,
    pub player_slug: String,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub is_on_watchlist: bool,
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
    pub season_base: String,
    pub seasons: Option<SeasonStep>,
    pub items: Vec<PlayerMatchItem>,
}

pub struct PlayerMatchItem {
    pub season: LeagueSeason,
    pub date: String,
    pub time: String,
    pub opponent_slug: String,
    pub opponent_name: String,
    pub is_home: bool,
    pub competition_name: String,
    pub result: Option<PlayerMatchResult>,
}

pub struct PlayerMatchResult {
    pub match_id: String,
    pub home_goals: u8,
    pub away_goals: u8,
}

pub struct PlayerMatchesPage;

impl PlayerMatchesPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayerMatchesRequest>,
        Query(query): Query<PlayerMatchesQuery>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (player, team_opt, canonical) = match PlayerSlug::resolve_page(
            simulator_data,
            &route_params.player_slug,
            &route_params.lang,
            "/matches",
        )? {
            PlayerPage::Found {
                player,
                team,
                canonical_slug,
            } => (player, team, canonical_slug),
            PlayerPage::Redirect(r) => return Ok(r),
        };

        // Read off the match histories of the player's teams rather than the
        // current team's schedule — see `collector` for why.
        let mut items = PlayerMatchCollector::collect(simulator_data, &i18n, player, team_opt);
        // Only played matches are listed, so the newest season holding one is
        // already the season under way — or the one just finished while the new
        // campaign has yet to kick off.
        let seasons = SeasonStep::resolve(items.iter().map(|item| item.season), query.season, None);
        if let Some(step) = &seasons {
            items.retain(|item| item.season.opening_year == step.selected);
        }

        let season_base = format!("/{}/players/{}/matches", route_params.lang, canonical);
        let chrome = PlayerPageChrome::build(
            simulator_data,
            i18n,
            player,
            team_opt,
            &route_params.lang,
            &canonical,
            "matches",
            PlayerPageColors::Grey,
        )?;

        Ok(PlayerMatchesTemplate {
            css_version: chrome.css_version,
            computer_name: chrome.computer_name,
            cpu_brand: chrome.cpu_brand,
            cores_count: chrome.cores_count,
            title: chrome.title,
            sub_title_prefix: chrome.sub_title_prefix,
            sub_title_suffix: chrome.sub_title_suffix,
            sub_title: chrome.sub_title,
            sub_title_link: chrome.sub_title_link,
            sub_title_flag: chrome.sub_title_flag,
            header_color: chrome.header_color,
            foreground_color: chrome.foreground_color,
            menu_sections: chrome.menu_sections,
            i18n: chrome.i18n,
            lang: chrome.lang,
            active_tab: chrome.active_tab,
            player_id: chrome.player_id,
            player_slug: chrome.player_slug,
            club_id: chrome.club_id,
            is_on_loan: chrome.is_on_loan,
            is_injured: chrome.is_injured,
            is_unhappy: chrome.is_unhappy,
            is_force_match_selection: chrome.is_force_match_selection,
            is_on_watchlist: chrome.is_on_watchlist,
            events_count: chrome.events_count,
            interested_clubs_count: chrome.interested_clubs_count,
            awards_count: chrome.awards_count,
            news_count: chrome.news_count,
            season_base,
            seasons,
            items,
        }
        .into_response())
    }
}
