pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::league::Season;
use core::utils::FormattingUtils;
use chrono::Datelike;
use core::{ContractType, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerHistoryRequest {
    pub lang: String,
    pub player_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/history/index.html")]
pub struct PlayerHistoryTemplate {
    pub css_version: &'static str,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub team_slug: String,
    pub player_id: u32,
    pub items: Vec<PlayerHistorySeasonItem>,
    pub current_club: String,
    pub current_is_loan: bool,
    pub current_is_free_transfer: bool,
    pub current_transfer_fee: String,
    pub current_season: String,
    pub current: PlayerHistoryStats,
    pub current_country_code: String,
    pub current_country_name: String,
    pub current_country_slug: String,
    pub current_league_name: String,
    pub current_league_slug: String,
}

pub struct PlayerHistorySeasonItem {
    pub season: String,
    pub team_name: String,
    pub team_slug: String,
    pub is_loan: bool,
    pub is_free_transfer: bool,
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
                        // Try league_id first
                        let league = t.league_id
                            .and_then(|lid| {
                                country.leagues.leagues.iter().find(|l| l.id == lid)
                            })
                            // Fallback: find league that has this team in its table
                            .or_else(|| {
                                country.leagues.leagues.iter().find(|l| {
                                    l.table.rows.iter().any(|row| row.team_id == t.id)
                                })
                            })
                            // Fallback: use first league of the country
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
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let (player, team) = simulator_data
        .player_with_team(route_params.player_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Player with ID {} not found", route_params.player_id))
        })?;

    let neighbor_teams: Vec<(String, String)> = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    // Get transfer history for this player
    let country = simulator_data.country_by_club(team.club_id);

    let player_transfers: Vec<_> = country
        .map(|c| {
            c.transfer_market.transfer_history.iter()
                .filter(|t| t.player_id == player.id)
                .collect()
        })
        .unwrap_or_default();

    let is_loaned_in = country
        .map(|c| {
            c.transfer_market.transfer_history.iter()
                .any(|t| t.player_id == player.id
                    && t.to_club_id == team.club_id
                    && matches!(&t.transfer_type, core::transfers::TransferType::Loan(_)))
        })
        .unwrap_or(false);

    // Build team slug → location lookup cache
    let mut location_cache: std::collections::HashMap<String, TeamLocationInfo> = std::collections::HashMap::new();

    let mut items: Vec<PlayerHistorySeasonItem> = player
        .statistics_history
        .items
        .iter()
        .map(|item| {
            let season_str = match &item.season {
                Season::OneYear(y) => format!("{}", y),
                Season::TwoYear(y1, y2) => format!("{}/{}", y1, y2 % 100),
            };

            let season_start_year = match &item.season {
                Season::OneYear(y) => *y,
                Season::TwoYear(y1, _) => *y1,
            };

            let transfer_record = player_transfers.iter()
                .find(|t| {
                    t.to_team_name == item.team_name
                        && (season_start_year == t.season_year || season_start_year == t.season_year + 1)
                });

            let transfer_fee = transfer_record
                .map(|t| {
                    if t.fee.amount > 0.0 {
                        FormattingUtils::format_money(t.fee.amount)
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();

            let is_free_transfer = !item.is_loan && transfer_record
                .map(|t| matches!(t.transfer_type, core::transfers::TransferType::Free) || (matches!(t.transfer_type, core::transfers::TransferType::Permanent) && t.fee.amount <= 0.0))
                .unwrap_or(false);

            // Lookup country/league info
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

            PlayerHistorySeasonItem {
                season: season_str,
                team_name: item.team_name.clone(),
                team_slug: item.team_slug.clone(),
                is_loan: item.is_loan,
                is_free_transfer,
                transfer_fee,
                stats: PlayerHistoryStats {
                    played: item.statistics.played,
                    played_subs: item.statistics.played_subs,
                    goals: item.statistics.goals,
                    assists: item.statistics.assists,
                    player_of_the_match: item.statistics.player_of_the_match,
                    average_rating: format!("{:.2}", item.statistics.average_rating),
                },
                country_code: location.map(|l| l.country_code.clone()).unwrap_or_default(),
                country_name: location.map(|l| l.country_name.clone()).unwrap_or_default(),
                country_slug: location.map(|l| l.country_slug.clone()).unwrap_or_default(),
                league_name: location.map(|l| l.league_name.clone()).unwrap_or_default(),
                league_slug: location.map(|l| l.league_slug.clone()).unwrap_or_default(),
            }
        })
        .collect();

    // Most recent season first
    items.reverse();

    let current = PlayerHistoryStats {
        played: player.statistics.played,
        played_subs: player.statistics.played_subs,
        goals: player.statistics.goals,
        assists: player.statistics.assists,
        player_of_the_match: player.statistics.player_of_the_match,
        average_rating: format!("{:.2}", player.statistics.average_rating),
    };

    let title = format!("{} {}", player.full_name.first_name, player.full_name.last_name);

    let sim_date = simulator_data.date.date();
    let year = sim_date.year();
    let month = sim_date.month();
    let current_season = if month >= 7 {
        format!("{}/{}", year, (year + 1) % 100)
    } else {
        format!("{}/{}", year - 1, year % 100)
    };

    // Current team's country/league info
    let current_location = find_team_location(simulator_data, &team.slug);

    let current_is_loan = player.contract.as_ref()
        .map(|c| c.contract_type == ContractType::Loan)
        .unwrap_or(false)
        || is_loaned_in;

    // Current season transfer fee
    let current_season_year = if month >= 7 { year as u16 } else { (year - 1) as u16 };
    let current_transfer = player_transfers.iter()
        .find(|t| {
            t.to_team_name == team.name
                && (current_season_year == t.season_year || current_season_year == t.season_year + 1)
        });

    let current_transfer_fee = current_transfer
        .map(|t| {
            if t.fee.amount > 0.0 {
                FormattingUtils::format_money(t.fee.amount)
            } else {
                String::new()
            }
        })
        .unwrap_or_default();

    let current_is_free_transfer = !current_is_loan && current_transfer
        .map(|t| matches!(t.transfer_type, core::transfers::TransferType::Free) || (matches!(t.transfer_type, core::transfers::TransferType::Permanent) && t.fee.amount <= 0.0))
        .unwrap_or(false);

    Ok(PlayerHistoryTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: if team.team_type == core::TeamType::Main { String::new() } else { i18n.t(team.team_type.as_i18n_key()).to_string() },
        sub_title: team.name.clone(),
        sub_title_link: format!("/{}/teams/{}", &route_params.lang, &team.slug),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
        menu_sections: views::player_menu(&i18n, &route_params.lang, &neighbor_refs, &team.slug, &format!("/{}/teams/{}", &route_params.lang, &team.slug)),
        i18n,
        lang: route_params.lang.clone(),
        team_slug: team.slug.clone(),
        player_id: route_params.player_id,
        items,
        current_club: team.name.clone(),
        current_is_loan,
        current_is_free_transfer,
        current_transfer_fee,
        current_season,
        current,
        current_country_code: current_location.as_ref().map(|l| l.country_code.clone()).unwrap_or_default(),
        current_country_name: current_location.as_ref().map(|l| l.country_name.clone()).unwrap_or_default(),
        current_country_slug: current_location.as_ref().map(|l| l.country_slug.clone()).unwrap_or_default(),
        current_league_name: current_location.as_ref().map(|l| l.league_name.clone()).unwrap_or_default(),
        current_league_slug: current_location.as_ref().map(|l| l.league_slug.clone()).unwrap_or_default(),
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

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}
