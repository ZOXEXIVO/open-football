pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerDecisionsRequest {
    pub lang: String,
    pub player_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/decisions/index.html")]
pub struct PlayerDecisionsTemplate {
    pub css_version: &'static str,
    pub hostname: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub player_id: u32,
    pub club_id: u32,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub decisions: Vec<PlayerDecisionItem>,
}

pub struct PlayerDecisionItem {
    pub date: String,
    pub movement: String,
    pub decision: String,
    pub decided_by: String,
}

pub async fn player_decisions_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerDecisionsRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let active = simulator_data.player_with_team(route_params.player_id);
    let player = if let Some((p, _)) = active {
        p
    } else if let Some(p) = simulator_data.retired_player(route_params.player_id) {
        p
    } else {
        return Err(ApiError::NotFound(format!(
            "Player with ID {} not found",
            route_params.player_id
        )));
    };
    let team_opt = active.map(|(_, t)| t);

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let mut decisions: Vec<PlayerDecisionItem> = player
        .decision_history
        .items
        .iter()
        .map(|d| PlayerDecisionItem {
            date: d.date.format("%d.%m.%Y").to_string(),
            movement: d.movement.clone(),
            decision: d.decision.clone(),
            decided_by: d.decided_by.clone(),
        })
        .collect();

    // Most recent first
    decisions.reverse();

    let title = format!(
        "{} {}",
        player.full_name.display_first_name(),
        player.full_name.display_last_name()
    );

    Ok(PlayerDecisionsTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        hostname: &crate::common::default_handler::HOSTNAME,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt
            .map(|t| t.name.clone())
            .unwrap_or_else(|| "Retired".to_string()),
        sub_title_link: team_opt
            .map(|t| format!("/{}/teams/{}", &route_params.lang, &t.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.background.clone())
            })
            .unwrap_or_else(|| "#808080".to_string()),
        foreground_color: team_opt
            .and_then(|t| {
                simulator_data
                    .club(t.club_id)
                    .map(|c| c.colors.foreground.clone())
            })
            .unwrap_or_else(|| "#ffffff".to_string()),
        menu_sections: if let Some(team) = team_opt {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams {
                i18n: &i18n,
                lang: &route_params.lang,
                current_path: &current_path,
                country_name: cn,
                country_slug: cs,
            };
            views::team_menu(
                &mp,
                &neighbor_refs,
                &team.slug,
                &league_refs,
                team.team_type == core::TeamType::Main,
            )
        } else {
            Vec::new()
        },
        i18n,
        lang: route_params.lang.clone(),
        active_tab: "decisions",
        player_id: route_params.player_id,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        decisions,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let club_name = &club.name;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            (
                format!("{}  |  {}", club_name, i18n.t(team.team_type.as_i18n_key())),
                team.slug.clone(),
                team.reputation.world,
            )
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    let mut country_leagues: Vec<(u32, String, String)> = data
        .country_by_club(club_id)
        .map(|country| {
            country
                .leagues
                .leagues
                .iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
                .collect()
        })
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);

    Ok((
        teams
            .into_iter()
            .map(|(name, slug, _)| (name, slug))
            .collect(),
        country_leagues
            .into_iter()
            .map(|(_, name, slug)| (name, slug))
            .collect(),
    ))
}
