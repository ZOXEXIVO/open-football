pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::{PlayerStatusType, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerMatchesRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/matches/index.html")]
pub struct PlayerMatchesTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
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
    pub items: Vec<PlayerMatchItem>,
}

pub struct PlayerMatchItem {
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

pub async fn player_matches_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerMatchesRequest>,
) -> ApiResult<Response> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let (player, team_opt, canonical) = match resolve_player_page(
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

    let league = team_opt.and_then(|t| t.league_id.and_then(|id| simulator_data.league(id)));

    let schedule = team_opt
        .map(|team| {
            league
                .map(|l| l.schedule.get_matches_for_team(team.id))
                .unwrap_or_default()
        })
        .unwrap_or_default();

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

    let items: Vec<PlayerMatchItem> = if let Some(team) = team_opt {
        // League matches the player participated in
        let mut match_items: Vec<(chrono::NaiveDateTime, PlayerMatchItem)> = schedule
            .iter()
            .filter(|schedule_item| {
                if schedule_item.result.is_none() {
                    return false;
                }
                if let Some(l) = league {
                    if let Some(match_result) = l.matches.get(&schedule_item.id) {
                        if let Some(details) = &match_result.details {
                            return details.player_stats.contains_key(&player.id);
                        }
                    }
                }
                false
            })
            .map(|schedule_item| {
                let is_home = schedule_item.home_team_id == team.id;

                let home_team_data = simulator_data
                    .team_data(schedule_item.home_team_id)
                    .unwrap();
                let away_team_data = simulator_data
                    .team_data(schedule_item.away_team_id)
                    .unwrap();

                (
                    schedule_item.date,
                    PlayerMatchItem {
                        date: schedule_item.date.format("%d.%m.%Y").to_string(),
                        time: schedule_item.date.format("%H:%M").to_string(),
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
                        result: schedule_item.result.as_ref().map(|res| PlayerMatchResult {
                            match_id: schedule_item.id.clone(),
                            home_goals: res.home_team.get(),
                            away_goals: res.away_team.get(),
                        }),
                    },
                )
            })
            .collect();

        // Continental competition matches for this club
        let continental_matches = simulator_data.continental_matches_for_club(team.club_id);
        for (comp_name, home_club_id, away_club_id, date, match_id, match_result) in
            continental_matches
        {
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
                .unwrap_or_else(|| ("Unknown".to_string(), String::new()));

            let datetime = date.and_hms_opt(20, 0, 0).unwrap();

            match_items.push((
                datetime,
                PlayerMatchItem {
                    date: date.format("%d.%m.%Y").to_string(),
                    time: "20:00".to_string(),
                    opponent_slug,
                    opponent_name,
                    is_home,
                    competition_name: comp_name.to_string(),
                    result: match_result.map(|(home_goals, away_goals)| PlayerMatchResult {
                        match_id: match_id.to_string(),
                        home_goals,
                        away_goals,
                    }),
                },
            ));
        }

        // National team competition matches (qualifying, tournament)
        if let Some(country) = simulator_data.country_by_club(team.club_id) {
            let is_in_squad = country
                .national_team
                .squad
                .iter()
                .any(|s| s.player_id == player.id);
            if is_in_squad || player.player_attributes.international_apps > 0 {
                for fixture in &country.national_team.schedule {
                    if let Some(ref result) = fixture.result {
                        let datetime = fixture.date.and_hms_opt(20, 0, 0).unwrap();
                        match_items.push((
                            datetime,
                            PlayerMatchItem {
                                date: fixture.date.format("%d.%m.%Y").to_string(),
                                time: "20:00".to_string(),
                                opponent_slug: String::new(),
                                opponent_name: fixture.opponent_country_name.clone(),
                                is_home: fixture.is_home,
                                competition_name: fixture.competition_name.clone(),
                                result: Some(PlayerMatchResult {
                                    match_id: fixture.match_id.clone(),
                                    home_goals: result.home_score,
                                    away_goals: result.away_score,
                                }),
                            },
                        ));
                    }
                }
            }
        }

        // Sort all matches by date
        match_items.sort_by_key(|(dt, _)| *dt);
        match_items.into_iter().map(|(_, item)| item).collect()
    } else {
        Vec::new()
    };

    let title = format!(
        "{} {}",
        player.full_name.display_first_name(),
        player.full_name.display_last_name()
    );

    Ok(PlayerMatchesTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: String::new(),
        sub_title: team_opt.map(|t| t.name.clone()).unwrap_or_else(|| {
            if player.is_retired() {
                i18n.t("retired").to_string()
            } else {
                i18n.t("free_agent").to_string()
            }
        }),
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
        active_tab: "matches",
        player_id: player.id,
        player_slug: canonical,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
        is_force_match_selection: player.is_force_match_selection,
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        items,
    }
    .into_response())
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
) -> Result<(Vec<(String, String)>, Vec<(String, String)>), ApiError> {
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
