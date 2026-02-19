pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamHistoryRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/history/index.html")]
pub struct TeamHistoryTemplate {
    pub css_version: &'static str,
    pub i18n: crate::I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub team_slug: String,
    pub items: Vec<TeamHistoryMatchItem>,
}

pub struct TeamHistoryMatchItem {
    pub date: String,
    pub opponent_name: String,
    pub home_goals: u8,
    pub away_goals: u8,
}

pub async fn team_history_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamHistoryRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

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

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let neighbor_teams: Vec<(String, String)> = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let items: Vec<TeamHistoryMatchItem> = team
        .match_history
        .items()
        .iter()
        .map(|item| {
            let opponent_name = simulator_data
                .team_data(item.rival_team_id)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| "Unknown".to_string());

            TeamHistoryMatchItem {
                date: item.date.format("%d.%m.%Y").to_string(),
                opponent_name,
                home_goals: item.score.0.get(),
                away_goals: item.score.1.get(),
            }
        })
        .collect();

    let menu_sections = views::team_menu(&i18n, &route_params.lang, &neighbor_refs, &team.slug, &format!("/{}/teams/{}/history", &route_params.lang, &team.slug));
    let title = if team.team_type == core::TeamType::Main { team.name.clone() } else { format!("{} - {}", team.name, i18n.t(team.team_type.as_i18n_key())) };

    Ok(TeamHistoryTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league.map(|l| l.name.clone()).unwrap_or_default(),
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.primary.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.secondary.clone()).unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        items,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<Vec<(String, String)>, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| (i18n.t(team.team_type.as_i18n_key()).to_string(), team.slug.clone(), team.reputation.world))
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams.into_iter().map(|(name, slug, _)| (name, slug)).collect())
}
