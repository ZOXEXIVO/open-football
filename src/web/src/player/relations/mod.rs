pub mod routes;

use crate::player::{PlayerPageChrome, PlayerPageColors};
use crate::shared::slug::{PlayerPage, PlayerSlug};
use crate::teams::relations::{RelNode, RelationsGraph, RelationsGroup};
use crate::views::{MenuSection, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerRelationsRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/relations/index.html")]
pub struct PlayerRelationsTemplate {
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
    pub watch_key: String,
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
    /// The subject player (node 0, `is_root`) plus every teammate they share
    /// a bond / friendship / tension / rivalry with.
    pub nodes: Vec<RelNode>,
    /// Edges serialised for the client-side force layout.
    pub edges_json: String,
    pub bond_count: usize,
    pub friendly_count: usize,
    pub tension_count: usize,
    pub rivalry_count: usize,
}

pub struct PlayerRelationsPage;

impl PlayerRelationsPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayerRelationsRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let world = state.world.snapshot();

        let simulator_data = world
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (player, team_opt, canonical) = match PlayerSlug::resolve_page(
            simulator_data,
            &route_params.player_slug,
            &route_params.lang,
            "/relations",
        )? {
            PlayerPage::Found {
                player,
                team,
                canonical_slug,
            } => (player, team, canonical_slug),
            PlayerPage::Redirect(r) => return Ok(r),
        };

        // Ego graph centred on this player: pool the whole dressing-room web the
        // player trains in, then keep only the player and their direct relations.
        // A free agent / retired player has no squad, so the graph is empty.
        let graph = match team_opt {
            Some(team) => {
                let pool = RelationsGroup::collect_pool(team, simulator_data);
                RelationsGraph::build_ego(&pool, player.id)
            }
            None => RelationsGraph::build_ego(&[], player.id),
        };

        let interested_clubs_count = simulator_data.clubs_interested_in_player(player.id).len();
        let chrome = PlayerPageChrome::build(
            simulator_data,
            i18n,
            player,
            team_opt,
            &route_params.lang,
            &canonical,
            "relations",
            PlayerPageColors::Grey,
        )?;

        Ok(PlayerRelationsTemplate {
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
            watch_key: chrome.watch_key,
            events_count: chrome.events_count,
            interested_clubs_count,
            awards_count: chrome.awards_count,
            news_count: chrome.news_count,
            edges_json: serde_json::to_string(&graph.edges).unwrap_or_else(|_| "[]".to_string()),
            bond_count: graph.bond_count,
            friendly_count: graph.friendly_count,
            tension_count: graph.tension_count,
            rivalry_count: graph.rivalry_count,
            nodes: graph.nodes,
        }
        .into_response())
    }
}
