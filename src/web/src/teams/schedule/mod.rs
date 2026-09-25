pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::played_fixture::PlayedFixture;
use crate::common::season_step::SeasonStep;
use crate::teams::newspaper::NewspaperCounter;
use crate::views::{self, MenuSection, NeighborMenus};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::{NaiveDate, NaiveDateTime};
use core::league::season::{LeagueSeason, Season};
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamScheduleGetRequest {
    lang: String,
    team_slug: String,
}

#[derive(Deserialize)]
pub struct TeamScheduleQuery {
    pub season: Option<i32>,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/schedule/index.html")]
pub struct TeamScheduleTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
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
    pub team_slug: String,
    pub active_tab: &'static str,
    pub show_finances_tab: bool,
    pub show_academy_tab: bool,
    /// Printed items waiting on the newspaper tab, for the tabbar badge.
    pub newspaper_count: usize,
    pub season_base: String,
    pub seasons: Option<SeasonStep>,
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
    Query(query): Query<TeamScheduleQuery>,
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

    // Continental fixtures have no calendar of their own: they are part of
    // the club's domestic campaign, so they file on the team's league too.
    let calendar = league.map(|l| l.settings.season_calendar());
    let campaign = |date: NaiveDate| {
        calendar
            .map(|c| c.season_of(date))
            .unwrap_or_else(|| Season::from_date(date).as_league_season())
    };

    // Everything played, every season, from the team's own history; the
    // schedules below only contribute what is still to come.
    let mut items: Vec<(NaiveDateTime, LeagueSeason, TeamScheduleItem)> = team
        .match_history
        .items()
        .iter()
        .map(|played| {
            let fixture = PlayedFixture::read(simulator_data, &i18n, team, played);
            (
                fixture.kickoff,
                fixture
                    .season
                    .unwrap_or_else(|| campaign(fixture.kickoff.date())),
                TeamScheduleItem {
                    date: fixture.date,
                    time: fixture.time,
                    opponent_slug: fixture.opponent_slug,
                    opponent_name: fixture.opponent_name,
                    is_home: fixture.is_home,
                    competition_name: fixture.competition_name,
                    result: Some(TeamScheduleItemResult {
                        match_id: fixture.match_id,
                        home_goals: fixture.home_goals,
                        away_goals: fixture.away_goals,
                    }),
                },
            )
        })
        .collect();

    // The domestic cup and the playoffs are stored apart from `leagues`, but
    // for listing a team's fixtures they are all just competitions.
    let competitions = simulator_data
        .country_by_club(team.club_id)
        .into_iter()
        .flat_map(|country| {
            country
                .leagues
                .leagues
                .iter()
                .chain(country.domestic_cup.as_ref().map(|cup| &cup.league))
                .chain(country.playoffs.iter().map(|playoff| &playoff.league))
        });
    for competition in competitions {
        let competition_calendar = competition.settings.season_calendar();
        for fixture in competition.schedule.get_matches_for_team(team.id) {
            if fixture.result.is_some() {
                continue;
            }
            let is_home = fixture.home_team_id == team.id;
            let opponent_id = if is_home {
                fixture.away_team_id
            } else {
                fixture.home_team_id
            };
            let opponent = simulator_data.team_data(opponent_id).unwrap();

            items.push((
                fixture.date,
                competition_calendar.season_of(fixture.date.date()),
                TeamScheduleItem {
                    date: fixture.date.format("%d.%m.%Y").to_string(),
                    time: fixture.date.format("%H:%M").to_string(),
                    opponent_slug: opponent.slug.clone(),
                    opponent_name: opponent.name.clone(),
                    is_home,
                    competition_name: competition.name.clone(),
                    result: None,
                },
            ));
        }
    }

    // Continental fixtures are keyed by *club*, not by team, so every squad of
    // the club — B, Second, U18..U23 — matches the club id. Only the Main squad
    // actually enters the bracket, so gate on it: otherwise "Real Madrid U18"
    // shows the first team's Champions League programme as its own.
    let continental_matches = if team.team_type == core::TeamType::Main {
        simulator_data.continental_matches_for_club(team.club_id)
    } else {
        Vec::new()
    };
    for (comp_key, home_club_id, away_club_id, date, _, match_result) in continental_matches {
        if match_result.is_some() {
            continue;
        }
        let is_home = home_club_id == team.club_id;
        let opponent_club_id = if is_home { away_club_id } else { home_club_id };

        let (opponent_name, opponent_slug) = simulator_data
            .club(opponent_club_id)
            .and_then(|club| {
                club.teams
                    .main_team_id()
                    .and_then(|tid| simulator_data.team(tid))
                    .map(|t| (t.name.clone(), t.slug.clone()))
            })
            .unwrap_or_else(|| (i18n.t("unknown").to_string(), String::new()));

        items.push((
            date.and_hms_opt(20, 0, 0).unwrap(),
            campaign(date),
            TeamScheduleItem {
                date: date.format("%d.%m.%Y").to_string(),
                time: "20:00".to_string(),
                opponent_slug,
                opponent_name,
                is_home,
                competition_name: i18n.t(comp_key).to_string(),
                result: None,
            },
        ));
    }

    // Sort all matches by date
    items.sort_by_key(|(dt, _, _)| *dt);
    let seasons = SeasonStep::resolve(
        items.iter().map(|(_, season, _)| *season),
        query.season,
        calendar.map(|c| c.season_of(simulator_data.date.date()).opening_year),
    );
    if let Some(step) = &seasons {
        items.retain(|(_, season, _)| season.opening_year == step.selected);
    }
    let items: Vec<TeamScheduleItem> = items.into_iter().map(|(_, _, item)| item).collect();

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/schedule", route_params.lang, team.slug);
    let menu_params = views::MenuParams {
        i18n: &i18n,
        lang: &route_params.lang,
        current_path: &current_path,
        country_name: cn,
        country_slug: cs,
    };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &league_refs);
    let title = team.name.clone();
    let league_title = league
        .map(|l| views::league_display_name(l, &i18n, simulator_data))
        .unwrap_or_default();

    Ok(TeamScheduleTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league
            .map(|l| format!("/{}/leagues/{}", route_params.lang, l.slug))
            .unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "schedule",
        show_finances_tab: team.team_type.is_own_team(),
        show_academy_tab: team.team_type == core::TeamType::Main
            || team.team_type == core::TeamType::U18,
        newspaper_count: NewspaperCounter::count(simulator_data, team),
        season_base: current_path,
        seasons,
        items,
    })
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<NeighborMenus, ApiError> {
    let club = data
        .club(club_id)
        .ok_or_else(|| ApiError::InternalError(format!("Club with ID {} not found", club_id)))?;

    let teams = views::neighbor_teams(club, i18n);

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
        teams,
        country_leagues
            .into_iter()
            .map(|(_, name, slug)| (name, slug))
            .collect(),
    ))
}
