pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::utils::FormattingUtils;
use chrono::{Datelike, NaiveDate};
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
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,
    pub i18n: crate::I18n,
    pub lang: String,
    pub active_tab: &'static str,
    pub team_slug: String,
    pub player_id: u32,
    pub items: Vec<PlayerHistorySeasonItem>,
    pub current_club: String,
    pub current_is_loan: bool,
    pub current_transfer_fee: String,
    pub current_season: String,
    pub current: PlayerHistoryStats,
    pub current_country_code: String,
    pub current_country_name: String,
    pub current_country_slug: String,
    pub current_league_name: String,
    pub current_league_slug: String,
    pub is_goalkeeper: bool,
    pub is_retired: bool,
}

pub struct PlayerHistorySeasonItem {
    pub season: String,
    pub start_year: u16,
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
    pub created_at: NaiveDate,
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

    // Try active player first, fall back to retired
    let active = simulator_data.player_with_team(route_params.player_id);
    let retired_player;
    let (player, team_opt): (&core::Player, Option<&core::Team>) = if let Some((p, t)) = active {
        (p, Some(t))
    } else if let Some(p) = simulator_data.retired_player(route_params.player_id) {
        retired_player = p;
        (retired_player, None)
    } else {
        return Err(ApiError::NotFound(format!("Player with ID {} not found", route_params.player_id)));
    };

    let is_retired = team_opt.is_none();

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let is_loaned_in = team_opt.and_then(|team| {
        simulator_data.country_by_club(team.club_id)
            .map(|c| {
                c.transfer_market.transfer_history.iter()
                    .any(|t| t.player_id == player.id
                        && t.to_club_id == team.club_id
                        && matches!(&t.transfer_type, core::transfers::TransferType::Loan(_)))
            })
    }).unwrap_or(false);

    // Build team slug → location lookup cache
    let mut location_cache: std::collections::HashMap<String, TeamLocationInfo> = std::collections::HashMap::new();

    // Group history items by (season, team_slug) — merge duplicates from
    // mid-season transfer snapshots + end-of-season snapshots.
    struct GroupKey {
        season_display: String,
        start_year: u16,
        team_slug: String,
        team_name: String,
        league_name: String,
        league_slug: String,
        is_loan: bool,
        transfer_fee: Option<f64>,
        created_at: NaiveDate,
    }
    struct GroupAccum {
        played: u16,
        played_subs: u16,
        goals: u16,
        assists: u16,
        player_of_the_match: u8,
        rating_sum: f32,
        rating_count: u16,
        conceded: u16,
        clean_sheets: u16,
    }

    let mut grouped: Vec<(GroupKey, GroupAccum)> = Vec::new();

    for item in &player.statistics_history.items {
        let key_match = grouped.iter().position(|(k, _)| {
            k.start_year == item.season.start_year
                && k.team_slug == item.team_slug
                && k.is_loan == item.is_loan
        });

        let games = item.statistics.total_games();

        if let Some(idx) = key_match {
            let (key, accum) = &mut grouped[idx];
            accum.played += item.statistics.played;
            accum.played_subs += item.statistics.played_subs;
            accum.goals += item.statistics.goals;
            accum.assists += item.statistics.assists;
            accum.player_of_the_match += item.statistics.player_of_the_match;
            accum.rating_sum += item.statistics.average_rating * games as f32;
            accum.rating_count += games;
            accum.conceded += item.statistics.conceded;
            accum.clean_sheets += item.statistics.clean_sheets;
            // Preserve transfer info from the entry that has it
            if item.transfer_fee.is_some() && key.transfer_fee.is_none() {
                key.transfer_fee = item.transfer_fee;
            }
            // Update league info from most recent entry
            // (handles league changes from promotion/relegation)
            if item.created_at > key.created_at {
                if !item.league_name.is_empty() {
                    key.league_name = item.league_name.clone();
                    key.league_slug = item.league_slug.clone();
                }
                // Only update created_at if this entry has actual games;
                // 0-game entries from season snapshots have later dates
                // that would corrupt sort order for loan spells
                if games > 0 {
                    key.created_at = item.created_at;
                }
            }
        } else {
            grouped.push((
                GroupKey {
                    season_display: item.season.display.clone(),
                    start_year: item.season.start_year,
                    team_slug: item.team_slug.clone(),
                    team_name: item.team_name.clone(),
                    league_name: item.league_name.clone(),
                    league_slug: item.league_slug.clone(),
                    is_loan: item.is_loan,
                    transfer_fee: item.transfer_fee,
                    created_at: item.created_at,
                },
                GroupAccum {
                    played: item.statistics.played,
                    played_subs: item.statistics.played_subs,
                    goals: item.statistics.goals,
                    assists: item.statistics.assists,
                    player_of_the_match: item.statistics.player_of_the_match,
                    rating_sum: item.statistics.average_rating * games as f32,
                    rating_count: games,
                    conceded: item.statistics.conceded,
                    clean_sheets: item.statistics.clean_sheets,
                },
            ));
        }
    }

    let mut items: Vec<PlayerHistorySeasonItem> = grouped
        .into_iter()
        .map(|(key, accum)| {
            // Lookup country info (country doesn't change with promotion/relegation)
            let location = if !key.team_slug.is_empty() {
                if !location_cache.contains_key(&key.team_slug) {
                    if let Some(info) = find_team_location(simulator_data, &key.team_slug) {
                        location_cache.insert(key.team_slug.clone(), info);
                    }
                }
                location_cache.get(&key.team_slug)
            } else {
                None
            };

            let avg_rating = if accum.rating_count > 0 {
                accum.rating_sum / accum.rating_count as f32
            } else {
                0.0
            };

            // If league name is empty (e.g. youth teams without own league),
            // fall back to the team's current league from location lookup
            let (league_name, league_slug) = if !key.league_name.is_empty() {
                (key.league_name, key.league_slug)
            } else {
                location
                    .map(|l| (l.league_name.clone(), l.league_slug.clone()))
                    .unwrap_or_default()
            };

            PlayerHistorySeasonItem {
                season: key.season_display,
                start_year: key.start_year,
                team_name: key.team_name,
                team_slug: key.team_slug,
                is_loan: key.is_loan,
                transfer_fee: match key.transfer_fee {
                    Some(f) if f > 0.0 => FormattingUtils::format_money(f),
                    Some(_) => "Free".to_string(),
                    None => String::new(),
                },
                stats: PlayerHistoryStats {
                    played: accum.played,
                    played_subs: accum.played_subs,
                    goals: accum.goals,
                    assists: accum.assists,
                    player_of_the_match: accum.player_of_the_match,
                    average_rating: core::PlayerStatistics::format_rating(avg_rating),
                    conceded: accum.conceded,
                    clean_sheets: accum.clean_sheets,
                },
                country_code: location.map(|l| l.country_code.clone()).unwrap_or_default(),
                country_name: location.map(|l| l.country_name.clone()).unwrap_or_default(),
                country_slug: location.map(|l| l.country_slug.clone()).unwrap_or_default(),
                league_name,
                league_slug,
                created_at: key.created_at,
            }
        })
        .collect();

    // Most recent first: sort by season year descending, then by created_at
    // descending within same season. When created_at is equal (mid-season
    // transfer creates selling + buying entries on the same date), show the
    // new club (the one with transfer fee info) first.
    items.sort_by(|a, b| {
        b.start_year.cmp(&a.start_year)
            .then(b.created_at.cmp(&a.created_at))
            .then_with(|| {
                let a_has_fee = !a.transfer_fee.is_empty();
                let b_has_fee = !b.transfer_fee.is_empty();
                b_has_fee.cmp(&a_has_fee)
            })
    });

    let title = format!("{} {}", player.full_name.display_first_name(), player.full_name.display_last_name());

    let sim_date = simulator_data.date.date();
    let year = sim_date.year();
    let month = sim_date.month();
    let current_season = if month >= 7 {
        format!("{}/{}", year, (year + 1) % 100)
    } else {
        format!("{}/{}", year - 1, year % 100)
    };

    if is_retired {
        // Retired player: show only history, no current season row
        Ok(PlayerHistoryTemplate {
            css_version: crate::common::default_handler::CSS_VERSION,
            title,
            sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
            sub_title_suffix: String::new(),
            sub_title: "Retired".to_string(),
            sub_title_link: String::new(),
            sub_title_country_code: String::new(),
            header_color: "#808080".to_string(),
            foreground_color: "#ffffff".to_string(),
            menu_sections: Vec::new(),
            i18n,
            lang: route_params.lang.clone(),
            active_tab: "history",
            team_slug: String::new(),
            player_id: route_params.player_id,
            items,
            current_club: String::new(),
            current_is_loan: false,
            current_transfer_fee: String::new(),
            current_season,
            current: PlayerHistoryStats {
                played: 0,
                played_subs: 0,
                goals: 0,
                assists: 0,
                player_of_the_match: 0,
                average_rating: String::new(),
                conceded: 0,
                clean_sheets: 0,
            },
            current_country_code: String::new(),
            current_country_name: String::new(),
            current_country_slug: String::new(),
            current_league_name: String::new(),
            current_league_slug: String::new(),
            is_goalkeeper: player.position().is_goalkeeper(),
            is_retired: true,
        })
    } else {
        let team = team_opt.unwrap();

        let current = PlayerHistoryStats {
            played: player.statistics.played,
            played_subs: player.statistics.played_subs,
            goals: player.statistics.goals,
            assists: player.statistics.assists,
            player_of_the_match: player.statistics.player_of_the_match,
            average_rating: player.statistics.average_rating_str(),
            conceded: player.statistics.conceded,
            clean_sheets: player.statistics.clean_sheets,
        };

        // For non-main teams, resolve main team's slug and league info
        let main_team_slug = if team.team_type != core::TeamType::Main {
            simulator_data.club(team.club_id)
                .and_then(|c| c.teams.teams.iter().find(|t| t.team_type == core::TeamType::Main))
                .map(|t| t.slug.clone())
        } else {
            None
        };
        let current_display_slug = main_team_slug.as_deref().unwrap_or(&team.slug);

        // Current team's country/league info (use main team for non-main teams)
        let current_location = find_team_location(simulator_data, current_display_slug);

        let current_is_loan = player.contract.as_ref()
            .map(|c| c.contract_type == ContractType::Loan)
            .unwrap_or(false)
            || is_loaned_in;

        // Current season transfer info from history entries
        let current_season_year = if month >= 7 { year as u16 } else { (year - 1) as u16 };
        let current_history_entry = player.statistics_history.items.iter()
            .find(|h| h.season.start_year == current_season_year && h.team_slug == current_display_slug);

        let current_transfer_fee = match current_history_entry.and_then(|h| h.transfer_fee) {
            Some(f) if f > 0.0 => FormattingUtils::format_money(f),
            Some(_) => "Free".to_string(),
            None => String::new(),
        };

        // Remove the current team's current-season entry from history items —
        // it is already shown as the hardcoded "current" row at the top.
        items.retain(|item| {
            !(item.start_year == current_season_year && item.team_slug == current_display_slug)
        });

        Ok(PlayerHistoryTemplate {
            css_version: crate::common::default_handler::CSS_VERSION,
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
            team_slug: current_display_slug.to_string(),
            player_id: route_params.player_id,
            items,
            current_club: simulator_data.club(team.club_id).map(|c| c.name.clone()).unwrap_or_else(|| team.name.clone()),
            current_is_loan,
            current_transfer_fee,
            current_season,
            current,
            current_country_code: current_location.as_ref().map(|l| l.country_code.clone()).unwrap_or_default(),
            current_country_name: current_location.as_ref().map(|l| l.country_name.clone()).unwrap_or_default(),
            current_country_slug: current_location.as_ref().map(|l| l.country_slug.clone()).unwrap_or_default(),
            current_league_name: current_location.as_ref().map(|l| l.league_name.clone()).unwrap_or_default(),
            current_league_slug: current_location.as_ref().map(|l| l.league_slug.clone()).unwrap_or_default(),
            is_goalkeeper: player.position().is_goalkeeper(),
            is_retired: false,
        })
    }
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
