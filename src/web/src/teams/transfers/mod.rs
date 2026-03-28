pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::Datelike;
use core::transfers::TransferType;
use core::utils::FormattingUtils;
use core::SimulatorData;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct TeamTransfersRequest {
    lang: String,
    team_slug: String,
}

#[derive(Deserialize)]
pub struct SeasonQuery {
    pub season: Option<u16>,
}

pub struct SeasonOption {
    pub year: u16,
    pub display: String,
    pub selected: bool,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "teams/transfers/index.html")]
pub struct TeamTransfersTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
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
    pub items: Vec<TransferListItem>,
    pub incoming_transfers: Vec<TransferHistoryItem>,
    pub outgoing_transfers: Vec<TransferHistoryItem>,
    pub incoming_loans: Vec<LoanHistoryItem>,
    pub outgoing_loans: Vec<LoanHistoryItem>,
    pub seasons: Vec<SeasonOption>,
}

pub struct TransferListItem {
    pub player_id: u32,
    pub player_name: String,
    pub position: String,
    pub value: String,
}

pub struct TransferHistoryItem {
    pub player_id: u32,
    pub player_name: String,
    pub other_team: String,
    pub other_team_slug: String,
    pub fee: String,
    pub date: String,
}

pub struct LoanHistoryItem {
    pub player_id: u32,
    pub player_name: String,
    pub other_team: String,
    pub other_team_slug: String,
    pub date: String,
    pub end_date: String,
}

pub async fn team_transfers_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<TeamTransfersRequest>,
    Query(query): Query<SeasonQuery>,
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

    let country = simulator_data
        .country_by_club(team.club_id)
        .ok_or_else(|| {
            ApiError::NotFound("Country not found for team".to_string())
        })?;

    let now = simulator_data.date.date();
    let (neighbor_teams, country_leagues) = get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let club_id = team.club_id;

    // Compute season options
    let sim_date = simulator_data.date.date();
    let current_season_year = if sim_date.month() >= 8 {
        sim_date.year() as u16
    } else {
        (sim_date.year() - 1) as u16
    };

    let selected_season = query.season.unwrap_or(current_season_year);

    let min_season_year = country
        .transfer_market
        .transfer_history
        .iter()
        .map(|t| t.season_year)
        .min()
        .unwrap_or(current_season_year);

    let seasons: Vec<SeasonOption> = (min_season_year..=current_season_year)
        .rev()
        .map(|y| SeasonOption {
            year: y,
            display: format!("{}/{}", y, (y + 1) % 100),
            selected: y == selected_season,
        })
        .collect();

    // Current transfer list items
    let items: Vec<TransferListItem> = team
        .transfer_list
        .items()
        .iter()
        .filter_map(|ti| {
            let player = team.players().into_iter().find(|p| p.id == ti.player_id)?;
            Some(TransferListItem {
                player_id: player.id,
                player_name: format!(
                    "{} {}",
                    player.full_name.display_first_name(), player.full_name.display_last_name()
                ),
                position: player.position().get_short_name().to_string(),
                value: FormattingUtils::format_money(player.value(
                    now,
                    league.map(|l| l.reputation).unwrap_or(0),
                    team.reputation.world,
                )),
            })
        })
        .collect();

    // Incoming transfers (players bought by this club, excluding loans)
    let incoming_transfers: Vec<TransferHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.season_year == selected_season && t.to_club_id == club_id && !matches!(t.transfer_type, TransferType::Loan(_)))
        .map(|t| {
            let other_team_slug = get_first_team_slug(country, t.from_club_id);
            TransferHistoryItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                other_team: t.from_team_name.clone(),
                other_team_slug,
                fee: if t.fee.amount > 0.0 { FormattingUtils::format_money(t.fee.amount) } else { "Free".to_string() },
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
            }
        })
        .collect();

    // Outgoing transfers (players sold by this club, excluding loans)
    let outgoing_transfers: Vec<TransferHistoryItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| t.season_year == selected_season && t.from_club_id == club_id && !matches!(t.transfer_type, TransferType::Loan(_)))
        .map(|t| {
            let other_team_slug = get_first_team_slug(country, t.to_club_id);
            TransferHistoryItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                other_team: t.to_team_name.clone(),
                other_team_slug,
                fee: if t.fee.amount > 0.0 { FormattingUtils::format_money(t.fee.amount) } else { "Free".to_string() },
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
            }
        })
        .collect();

    // Incoming loans: players on this team with a loan contract
    let incoming_loans: Vec<LoanHistoryItem> = team
        .players()
        .iter()
        .filter_map(|p| {
            let loan_contract = p.contract_loan.as_ref()?;
            let from_club_id = loan_contract.loan_from_club_id?;
            let from_club = simulator_data.club(from_club_id);
            let from_team_name = from_club
                .and_then(|c| c.teams.teams.first())
                .map(|t| t.name.clone())
                .unwrap_or_default();
            let from_team_slug = from_club
                .and_then(|c| c.teams.teams.first())
                .map(|t| t.slug.clone())
                .unwrap_or_default();

            Some(LoanHistoryItem {
                player_id: p.id,
                player_name: format!(
                    "{} {}",
                    p.full_name.display_first_name(),
                    p.full_name.display_last_name()
                ),
                other_team: from_team_name,
                other_team_slug: from_team_slug,
                date: p.last_transfer_date
                    .map(|d| d.format("%d.%m.%Y").to_string())
                    .unwrap_or_default(),
                end_date: loan_contract.expiration.format("%d.%m.%Y").to_string(),
            })
        })
        .collect();

    // Outgoing loans: players on other teams whose contract_loan has loan_from_club_id == this club
    let mut outgoing_loans: Vec<LoanHistoryItem> = Vec::new();
    for continent in &simulator_data.continents {
        for country_iter in &continent.countries {
            for club in &country_iter.clubs {
                for team_iter in &club.teams.teams {
                    for player in &team_iter.players.players {
                        let is_loaned_from_us = player.contract_loan.as_ref().map(|c| {
                            c.loan_from_club_id == Some(club_id)
                        }).unwrap_or(false);

                        if !is_loaned_from_us { continue; }

                        let contract = player.contract_loan.as_ref().unwrap();
                        outgoing_loans.push(LoanHistoryItem {
                            player_id: player.id,
                            player_name: format!(
                                "{} {}",
                                player.full_name.display_first_name(),
                                player.full_name.display_last_name()
                            ),
                            other_team: team_iter.name.clone(),
                            other_team_slug: team_iter.slug.clone(),
                            date: player.last_transfer_date
                                .map(|d| d.format("%d.%m.%Y").to_string())
                                .unwrap_or_default(),
                            end_date: contract.expiration.format("%d.%m.%Y").to_string(),
                        });
                    }
                }
            }
        }
    }

    let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
    let current_path = format!("/{}/teams/{}/transfers", &route_params.lang, &team.slug);
    let menu_params = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
    let menu_sections = views::team_menu(&menu_params, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main);
    let title = team.name.clone();
    let league_title = league.map(|l| views::league_display_name(l, &i18n, simulator_data)).unwrap_or_default();

    Ok(TeamTransfersTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        computer_name: &crate::common::default_handler::COMPUTER_NAME,
        i18n,
        lang: route_params.lang.clone(),
        title,
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: league_title,
        sub_title_link: league.map(|l| format!("/{}/leagues/{}", &route_params.lang, &l.slug)).unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: simulator_data.club(team.club_id).map(|c| c.colors.background.clone()).unwrap_or_default(),
        foreground_color: simulator_data.club(team.club_id).map(|c| c.colors.foreground.clone()).unwrap_or_default(),
        menu_sections,
        team_slug: team.slug.clone(),
        active_tab: "transfers",
        show_finances_tab: team.team_type == core::TeamType::Main || team.team_type == core::TeamType::B,
        show_academy_tab: team.team_type == core::TeamType::Main || team.team_type == core::TeamType::U18,
        items,
        incoming_transfers,
        outgoing_transfers,
        incoming_loans,
        outgoing_loans,
        seasons,
    })
}

fn get_first_team_slug(country: &core::Country, club_id: u32) -> String {
    country
        .clubs
        .iter()
        .find(|c| c.id == club_id)
        .and_then(|c| c.teams.teams.first())
        .map(|t| t.slug.clone())
        .unwrap_or_default()
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
