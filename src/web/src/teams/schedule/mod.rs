pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamScheduleGetRequest {
    lang: String,
    team_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/schedule/index.html")]
pub struct TeamScheduleTemplate {
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
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    pub league_slug: String,
    pub items: Vec<TeamScheduleItem>,
}

pub struct TeamScheduleItem {
    pub date: String,
    pub time: String,
    pub opponent_slug: String,
    pub opponent_name: String,
    pub is_home: bool,
    pub competition_name: String,
    pub result: Option<TeamScheduleItemResult>,
}

pub struct TeamScheduleItemResult {
    pub match_id: String,
    pub home_goals: u8,
    pub away_goals: u8,
}

pub async fn team_schedule_get_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamScheduleGetRequest>,
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

    let league = team.league_id.and_then(|id| simulator_data.league(id));

    let schedule = league.map(|l| l.schedule.get_matches_for_team(team.id)).unwrap_or_default();

    let (neighbor_teams, country_leagues) = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    // League matches
    let mut items: Vec<(chrono::NaiveDateTime, TeamScheduleItem)> = schedule
        .iter()
        .map(|schedule| {
            let is_home = schedule.home_team_id == team.id;

            let home_team_data = simulator_data.team_data(schedule.home_team_id).unwrap();
            let away_team_data = simulator_data.team_data(schedule.away_team_id).unwrap();

            (schedule.date, TeamScheduleItem {
                date: schedule.date.format("%d.%m.%Y").to_string(),
                time: schedule.date.format("%H:%M").to_string(),
                opponent_slug: if is_home {
                    away_team_data.slug.clone()
                } else {
                    home_team_data.slug.clone()
                },
                opponent_name: if is_home {
                    away_team_data.name.clone()
                } else {
                    home_team_data.name.clone()
                },
                is_home,
                competition_name: league.map(|l| l.name.clone()).unwrap_or_default(),
                result: schedule.result.as_ref().map(|res| TeamScheduleItemResult {
                    match_id: schedule.id.clone(),
                    home_goals: res.home_team.get(),
                    away_goals: res.away_team.get(),
                }),
            })
        })
        .collect();

    // Continental competition matches (Champions League, Europa League, Conference League)
    let continental_matches = simulator_data.continental_matches_for_club(team.club_id);
    for (comp_name, home_club_id, away_club_id, date) in continental_matches {
        let is_home = home_club_id == team.club_id;
        let opponent_club_id = if is_home { away_club_id } else { home_club_id };

        let (opponent_name, opponent_slug) = simulator_data.club(opponent_club_id)
            .and_then(|club| {
                club.teams.main_team_id()
                    .and_then(|tid| simulator_data.team(tid))
                    .map(|t| (t.name.clone(), t.slug.clone()))
            })
            .unwrap_or_else(|| ("Unknown".to_string(), String::new()));

        let datetime = date.and_hms_opt(20, 0, 0).unwrap();

        items.push((datetime, TeamScheduleItem {
            date: date.format("%d.%m.%Y").to_string(),
            time: "20:00".to_string(),
            opponent_slug,
            opponent_name,
            is_home,
            competition_name: comp_name.to_string(),
            result: None, // Continental matches don't have results on ContinentalMatch
        }));
    }

    // Sort all matches by date
    items.sort_by_key(|(dt, _)| *dt);
    let items: Vec<TeamScheduleItem> = items.into_iter().map(|(_, item)| item).collect();

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/schedule", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main);
    let title = team.name.clone();
    let league_title = league.map(|l| views::league_display_name(l, &i18n, simulator_data)).unwrap_or_default();

    Ok(TeamScheduleTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        sub_title_country_code: simulator_data.country_by_club(team.club_id).map(|c| c.code.to_lowercase()).unwrap_or_default(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "schedule",
        show_finances_tab: team.team_type == core::TeamType::Main || team.team_type == core::TeamType::B,
        show_academy_tab: team.team_type == core::TeamType::Main || team.team_type == core::TeamType::U18,
        league_slug: league.map(|l| l.slug.clone()).unwrap_or_default(),
        items,
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
            (format!("{}  |  {}", club_name, i18n.t(team.team_type.as_i18n_key())), team.slug.clone(), team.reputation.world)
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    let mut country_leagues: Vec<(u32, String, String)> = data
        .country_by_club(club_id)
        .map(|country| {
            country.leagues.leagues.iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
                .collect()
        })
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);

    Ok((
        teams.into_iter().map(|(name, slug, _)| (name, slug)).collect(),
        country_leagues.into_iter().map(|(_, name, slug)| (name, slug)).collect(),
    ))
}
