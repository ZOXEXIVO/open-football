pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamStatsRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/stats/index.html")]
pub struct TeamStatsTemplate {
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
    pub players: Vec<TeamPlayerStats>,
}

pub struct TeamPlayerStats {
    pub id: u32,
    pub last_name: String,
    pub first_name: String,
    pub position: String,
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub assists: u16,
    pub yellow_cards: u8,
    pub red_cards: u8,
    pub shots_on_target: f32,
    pub passes: u8,
    pub tackling: f32,
    pub average_rating: String,
}

pub async fn team_stats_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamStatsRequest>,
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

    let (neighbor_teams, league_info) = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Option<(&str, &str)> = league_info.as_ref().map(|(n, s)| (n.as_str(), s.as_str()));

    let mut raw_players: Vec<(&core::Player, f32)> = team
        .players()
        .iter()
        .map(|p| (*p, p.statistics.average_rating))
        .collect();

    raw_players.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let players: Vec<TeamPlayerStats> = raw_players
        .iter()
        .map(|(p, _)| TeamPlayerStats {
            id: p.id,
            last_name: p.full_name.last_name.clone(),
            first_name: p.full_name.first_name.clone(),
            position: p.position().get_short_name().to_string(),
            played: p.statistics.played,
            played_subs: p.statistics.played_subs,
            goals: p.statistics.goals,
            assists: p.statistics.assists,
            yellow_cards: p.statistics.yellow_cards,
            red_cards: p.statistics.red_cards,
            shots_on_target: p.statistics.shots_on_target,
            passes: p.statistics.passes,
            tackling: p.statistics.tackling,
            average_rating: format!("{:.2}", p.statistics.average_rating),
        })
        .collect();

    let menu_sections = views::team_menu(&i18n, &route_params.lang, &neighbor_refs, &team.slug, &format!("/{}/teams/{}/stats", &route_params.lang, &team.slug), league_refs);
    let title = if team.team_type == core::TeamType::Main { team.name.clone() } else { format!("{} - {}", team.name, i18n.t(team.team_type.as_i18n_key())) };

    Ok(TeamStatsTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league.map(|l| l.name.clone()).unwrap_or_default(),
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        players,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &crate::I18n,
) -> Result<(Vec<(String, String)>, Option<(String, String)>), ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let club_name = &club.name;

    let mut league_info: Option<(String, String)> = None;

    let mut teams: Vec<(String, String, u16)> = club
        .teams
        .teams
        .iter()
        .map(|team| {
            if team.team_type == core::TeamType::Main {
                if let Some(league_id) = team.league_id {
                    if let Some(league) = data.league(league_id) {
                        league_info = Some((league.name.clone(), league.slug.clone()));
                    }
                }
            }
            (format!("{} {}", club_name, i18n.t(team.team_type.as_i18n_key())), team.slug.clone(), team.reputation.world)
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok((teams.into_iter().map(|(name, slug, _)| (name, slug)).collect(), league_info))
}
