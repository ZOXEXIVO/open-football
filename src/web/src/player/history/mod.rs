pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::common::slug::{resolve_player_page, PlayerPage};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::utils::FormattingUtils;
use core::{PlayerStatusType, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerHistoryRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/history/index.html")]
pub struct PlayerHistoryTemplate {
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
    pub items: Vec<PlayerHistorySeasonItem>,
    pub totals: PlayerHistoryStats,
    pub is_goalkeeper: bool,
    pub is_on_loan: bool,
    pub is_injured: bool,
    pub is_unhappy: bool,
    pub is_force_match_selection: bool,
    pub is_on_watchlist: bool,
}

pub struct PlayerHistorySeasonItem {
    pub season: String,
    pub team_name: String,
    pub team_slug: String,
    pub is_loan: bool,
    pub transfer_fee: String,
    pub stats: PlayerHistoryStats,
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub league_name: String,
    pub league_slug: String,
}

pub struct PlayerHistoryStats {
    pub played: u16,
    pub played_subs: u16,
    pub goals: u16,
    pub assists: u16,
    pub player_of_the_match: u8,
    pub average_rating: String,
    pub conceded: u16,
    pub clean_sheets: u16,
}

struct TeamLocationInfo {
    pub country_code: String,
    pub country_name: String,
    pub country_slug: String,
    pub league_name: String,
    pub league_slug: String,
}

fn find_team_location(simulator_data: &SimulatorData, team_slug: &str) -> Option<TeamLocationInfo> {
    for continent in &simulator_data.continents {
        for country in &continent.countries {
            for club in &country.clubs {
                for t in &club.teams.teams {
                    if t.slug == team_slug {
                        let league = t.league_id
                            .and_then(|lid| {
                                country.leagues.leagues.iter().find(|l| l.id == lid)
                            })
                            .or_else(|| {
                                country.leagues.leagues.iter().find(|l| {
                                    l.table.rows.iter().any(|row| row.team_id == t.id)
                                })
                            })
                            .or_else(|| {
                                country.leagues.leagues.first()
                            });

                        let (league_name, league_slug) = league
                            .map(|l| (l.name.clone(), l.slug.clone()))
                            .unwrap_or_default();

                        return Some(TeamLocationInfo {
                            country_code: country.code.clone(),
                            country_name: country.name.clone(),
                            country_slug: country.slug.clone(),
                            league_name,
                            league_slug,
                        });
                    }
                }
            }
        }
    }
    None
}

pub async fn player_history_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerHistoryRequest>,
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
        "/history",
    )? {
        PlayerPage::Found { player, team, canonical_slug } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

    let has_no_team = team_opt.is_none();

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let title = format!("{} {}", player.full_name.display_first_name(), player.full_name.display_last_name());

    // Pass live stats so the active entry gets current player.statistics
    let live_stats = if team_opt.is_some() {
        Some(&player.statistics)
    } else {
        None
    };
    let view = player.statistics_history.view_items(live_stats);
    let career_totals = core::PlayerStatisticsHistory::career_totals(&view);

    let mut location_cache: std::collections::HashMap<String, TeamLocationInfo> = std::collections::HashMap::new();

    let items: Vec<PlayerHistorySeasonItem> = view
        .into_iter()
        .map(|item| {
            let location = if !item.team_slug.is_empty() {
                if !location_cache.contains_key(&item.team_slug) {
                    if let Some(info) = find_team_location(simulator_data, &item.team_slug) {
                        location_cache.insert(item.team_slug.clone(), info);
                    }
                }
                location_cache.get(&item.team_slug)
            } else {
                None
            };

            // If league name is empty, fall back to team's current league
            let (league_name, league_slug) = if !item.league_name.is_empty() {
                (item.league_name, item.league_slug)
            } else {
                location
                    .map(|l| (l.league_name.clone(), l.league_slug.clone()))
                    .unwrap_or_default()
            };

            PlayerHistorySeasonItem {
                season: item.season.display,
                team_name: item.team_name,
                team_slug: item.team_slug,
                is_loan: item.is_loan,
                transfer_fee: match item.transfer_fee {
                    Some(f) if f > 0.0 => FormattingUtils::format_money(f),
                    Some(_) => "Free".to_string(),
                    None => String::new(),
                },
                stats: PlayerHistoryStats {
                    played: item.statistics.played,
                    played_subs: item.statistics.played_subs,
                    goals: item.statistics.goals,
                    assists: item.statistics.assists,
                    player_of_the_match: item.statistics.player_of_the_match,
                    average_rating: core::PlayerStatistics::format_rating(item.statistics.average_rating),
                    conceded: item.statistics.conceded,
                    clean_sheets: item.statistics.clean_sheets,
                },
                country_code: location.map(|l| l.country_code.clone()).unwrap_or_default(),
                country_name: location.map(|l| l.country_name.clone()).unwrap_or_default(),
                country_slug: location.map(|l| l.country_slug.clone()).unwrap_or_default(),
                league_name,
                league_slug,
            }
        })
        .collect();

    let totals = PlayerHistoryStats {
        played: career_totals.played,
        played_subs: career_totals.played_subs,
        goals: career_totals.goals,
        assists: career_totals.assists,
        player_of_the_match: career_totals.player_of_the_match,
        average_rating: core::PlayerStatistics::format_rating(career_totals.average_rating),
        conceded: career_totals.conceded,
        clean_sheets: career_totals.clean_sheets,
    };

    if has_no_team {
        let sub_title = if player.is_retired() {
            i18n.t("retired").to_string()
        } else {
            i18n.t("free_agent").to_string()
        };
        Ok(PlayerHistoryTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            title,
            sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
            sub_title_suffix: String::new(),
            sub_title,
            sub_title_link: String::new(),
            sub_title_country_code: String::new(),
            header_color: "#808080".to_string(),
            foreground_color: "#ffffff".to_string(),
            menu_sections: Vec::new(),
            i18n,
            lang: route_params.lang.clone(),
            active_tab: "history",
            player_id: player.id,
            player_slug: canonical.clone(),
            club_id: 0,
            items,
            totals,
            is_goalkeeper: player.position().is_goalkeeper(),
            is_on_loan: false,
            is_injured: false,
            is_unhappy: false,
            is_force_match_selection: player.is_force_match_selection,
            is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        }.into_response())
    } else {
        let team = team_opt.unwrap();

        Ok(PlayerHistoryTemplate {
            css_version: CSS_VERSION,
            computer_name: &COMPUTER_NAME,
            title,
            sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
            sub_title_suffix: String::new(),
            sub_title: team.name.clone(),
            sub_title_link: format!("/{}/teams/{}", &route_params.lang, &team.slug),
            sub_title_country_code: String::new(),
            header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
            foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
            menu_sections: {
                let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
                let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
                let mp = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
                views::team_menu(&mp, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main)
            },
            i18n,
            lang: route_params.lang.clone(),
            active_tab: "history",
            player_id: player.id,
            player_slug: canonical,
            club_id: team.club_id,
            items,
            totals,
            is_goalkeeper: player.position().is_goalkeeper(),
            is_on_loan: player.is_on_loan(),
            is_injured: player.player_attributes.is_injured,
            is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
            is_force_match_selection: player.is_force_match_selection,
            is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        }.into_response())
    }
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
            country.leagues.leagues.iter()
                .filter(|l| !l.friendly)
                .map(|l| (l.id, l.name.clone(), l.slug.clone()))
                .collect()
        })
        .unwrap_or_default();
    country_leagues.sort_by_key(|(id, _, _)| *id);

    Ok((
        teams,
        country_leagues.into_iter().map(|(_, name, slug)| (name, slug)).collect(),
    ))
}
