pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::PlayerPositionType;
use core::utils::DateUtils;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamAcademyRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/academy/index.html")]
#[allow(dead_code)]
pub struct TeamAcademyTemplate {
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
    pub team_slug: String,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    pub players: Vec<AcademyPlayer>,
}

pub struct AcademyPlayer {
    pub id: u32,
    pub first_name: String,
    pub last_name: String,
    pub position: String,
    pub position_sort: PlayerPositionType,
    pub country_slug: String,
    pub country_code: String,
    pub country_name: String,
    pub age: u8,
    pub current_ability: u8,
    pub potential_ability: u8,
    pub conditions: u8,
}

pub async fn team_academy_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamAcademyRequest>,
) -> ApiResult<impl IntoResponse> {
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let i18n = state.i18n.for_lang(&route_params.lang);

    let team_id = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?
        .slug_indexes
        .get_team_by_slug(&route_params.team_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Team '{}' not found", route_params.team_slug))
        })?;

    let team = simulator_data
        .team(team_id)
        .ok_or_else(|| ApiError::NotFound(format!("Team with ID {} not found", team_id)))?;

    // Academy tab only available for Main and U18 teams
    if team.team_type != core::TeamType::Main && team.team_type != core::TeamType::U18 {
        return Err(ApiError::NotFound(
            "Academy not available for this team type".to_string(),
        ));
    }

    let league = team.league_id.and_then(|id| simulator_data.league(id));
    let now = simulator_data.date.date();

    let club = simulator_data
        .club(team.club_id)
        .ok_or_else(|| {
            ApiError::InternalError(format!("Club with ID {} not found", team.club_id))
        })?;

    // Get academy players directly from club academy
    let mut players: Vec<AcademyPlayer> = club
        .academy
        .players
        .players
        .iter()
        .filter_map(|p| {
            let country = simulator_data.country(p.country_id)?;
            let position = p.positions.display_positions().join(", ");

            Some(AcademyPlayer {
                id: p.id,
                first_name: p.full_name.display_first_name().to_string(),
                last_name: p.full_name.display_last_name().to_string(),
                position,
                position_sort: p.position(),
                country_slug: country.slug.clone(),
                country_code: country.code.clone(),
                country_name: country.name.clone(),
                age: DateUtils::age(p.birth_date, now),
                current_ability: get_ability_stars(p.player_attributes.current_ability),
                potential_ability: get_ability_stars(p.player_attributes.potential_ability),
                conditions: (100f32 * (p.player_attributes.condition as f32 / 10000.0)) as u8,
            })
        })
        .collect();

    players.sort_by(|a, b| {
        a.position_sort
            .partial_cmp(&b.position_sort)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let (neighbor_teams, country_leagues) =
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

    let menu_sections = views::team_menu(
        &i18n,
        &route_params.lang,
        &neighbor_refs,
        &team.slug,
        &format!("/{}/teams/{}/academy", &route_params.lang, &team.slug),
        &league_refs,
        team.team_type == core::TeamType::Main,
    );

    let title = team.name.clone();

    Ok(TeamAcademyTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league.map(|l| l.name.clone()).unwrap_or_default(),
        sub_title_link: league
            .map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: club.colors.background.clone(),
        foreground_color: club.colors.foreground.clone(),
        menu_sections,
        team_slug: team.slug.clone(),
        show_finances_tab: team.team_type == core::TeamType::Main
            || team.team_type == core::TeamType::B,
        show_academy_tab: true,
        players,
    })
}

fn get_ability_stars(ability: u8) -> u8 {
    (5.0f32 * (ability as f32 / 200.0)).round() as u8
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
