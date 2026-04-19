pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::Datelike;
use core::transfers::TransferType;
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct LeagueTransfersRequest {
    lang: String,
    league_slug: String,
}

#[derive(Deserialize)]
pub struct SeasonQuery {
    pub season: Option<u16>,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/transfers/index.html")]
pub struct LeagueTransfersTemplate {
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
    pub league_slug: String,
    pub completed_transfers: Vec<CompletedTransferItem>,
    pub has_permanent_transfers: bool,
    pub has_loan_transfers: bool,

    pub active_negotiations: Vec<NegotiationItem>,
    pub seasons: Vec<SeasonOption>,
}

pub struct SeasonOption {
    pub year: u16,
    pub display: String,
    pub selected: bool,
}

pub struct CompletedTransferItem {
    pub player_id: u32,
    pub player_name: String,
    pub from_team: String,
    pub from_team_slug: String,
    pub to_team: String,
    pub to_team_slug: String,
    pub fee: String,
    pub is_loan: bool,
    pub date: String,
}


pub struct NegotiationItem {
    pub player_id: u32,
    pub player_name: String,
    pub selling_team: String,
    pub selling_team_slug: String,
    pub buying_team: String,
    pub buying_team_slug: String,
    pub offer_amount: String,
    pub status: String,
}

pub async fn league_transfers_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<LeagueTransfersRequest>,
    Query(query): Query<SeasonQuery>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let guard = state.data.read().await;

    let simulator_data = guard
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

    let indexes = simulator_data
        .indexes
        .as_ref()
        .ok_or_else(|| ApiError::InternalError("Indexes not available".to_string()))?;

    let league_id = indexes
        .slug_indexes
        .get_league_by_slug(&route_params.league_slug)
        .ok_or_else(|| {
            ApiError::NotFound(format!("League '{}' not found", route_params.league_slug))
        })?;

    let league = simulator_data
        .league(league_id)
        .ok_or_else(|| ApiError::NotFound(format!("League with ID {} not found", league_id)))?;

    let country = simulator_data
        .country(league.country_id)
        .ok_or_else(|| {
            ApiError::NotFound(format!("Country with ID {} not found", league.country_id))
        })?;

    // Get team IDs directly from the league table (precise per-league filter)
    let league_team_ids: Vec<u32> = league.table.get().iter().map(|row| row.team_id).collect();

    // Get club IDs for fallback (when from_team_id is 0, e.g. foreign transfers)
    let league_club_ids: Vec<u32> = league
        .table
        .get()
        .iter()
        .filter_map(|row| simulator_data.team(row.team_id).map(|t| t.club_id))
        .collect();

    // Compute current season year and available seasons
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

    // Completed transfers involving league teams, filtered by season.
    // Use from_team_id (specific team) when available; fall back to club_id for
    // foreign transfers (where from_team_id is 0) and for the buying side.
    let completed_transfers: Vec<CompletedTransferItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| {
            t.season_year == selected_season
                && t.from_club_id != 0
                && (league_team_ids.contains(&t.from_team_id)
                || league_club_ids.contains(&t.from_club_id)
                || league_club_ids.contains(&t.to_club_id))
        })
        .map(|t| {
            let from_team_slug = get_first_team_slug(simulator_data, country, t.from_club_id);
            let to_team_slug = get_first_team_slug(simulator_data, country, t.to_club_id);
            CompletedTransferItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                from_team: t.from_team_name.clone(),
                from_team_slug,
                to_team: t.to_team_name.clone(),
                to_team_slug,
                fee: if t.fee.amount > 0.0 { FormattingUtils::format_money(t.fee.amount) } else { "Free".to_string() },
                is_loan: matches!(&t.transfer_type, TransferType::Loan(_)),
                date: t.transfer_date.format("%d.%m.%Y").to_string(),
            }
        })
        .collect();

    // Active negotiations
    let active_negotiations: Vec<NegotiationItem> = country
        .transfer_market
        .negotiations
        .values()
        .filter(|n| {
            league_club_ids.contains(&n.selling_club_id)
                || league_club_ids.contains(&n.buying_club_id)
        })
        .filter(|n| {
            n.status == core::transfers::NegotiationStatus::Pending
                || n.status == core::transfers::NegotiationStatus::Countered
        })
        .filter_map(|n| {
            let selling_club = country.clubs.iter().find(|c| c.id == n.selling_club_id)?;
            let buying_club = country.clubs.iter().find(|c| c.id == n.buying_club_id)?;

            // Find the player name globally — the player may have moved to another country/continent
            let player_name = simulator_data
                .player(n.player_id)
                .map(|p| p.full_name.to_string())
                .unwrap_or_else(|| format!("Player #{}", n.player_id));

            let selling_team_slug = selling_club.teams.teams.first()
                .map(|t| t.slug.clone())
                .unwrap_or_default();
            let buying_team_slug = buying_club.teams.teams.first()
                .map(|t| t.slug.clone())
                .unwrap_or_default();
            Some(NegotiationItem {
                player_id: n.player_id,
                player_name,
                selling_team: selling_club.name.clone(),
                selling_team_slug,
                buying_team: buying_club.name.clone(),
                buying_team_slug,
                offer_amount: FormattingUtils::format_money(n.current_offer.base_fee.amount),
                status: format!("{:?}", n.status),
            })
        })
        .collect();

    let league_title = views::league_display_name(&league, &i18n, simulator_data);

    Ok(LeagueTransfersTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        title: format!("{} - Transfers", league_title),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: country.name.clone(),
        sub_title_link: format!("/{}/countries/{}", &route_params.lang, &country.slug),
        sub_title_country_code: country.code.clone(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: {
            let mut cl: Vec<(u32, &str, &str)> = country.leagues.leagues.iter().filter(|l| !l.friendly).map(|l| (l.id, l.name.as_str(), l.slug.as_str())).collect();
            cl.sort_by_key(|(id, _, _)| *id);
            let cl_refs: Vec<(&str, &str)> = cl.iter().map(|(_, n, s)| (*n, *s)).collect();
            let current_path = format!("/{}/leagues/{}/transfers", &route_params.lang, &league.slug);
            let mp = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: &country.name, country_slug: &country.slug };
            views::league_menu(&mp, &league.slug, &cl_refs)
        },
        league_slug: league.slug.clone(),
        has_permanent_transfers: completed_transfers.iter().any(|t| !t.is_loan),
        has_loan_transfers: completed_transfers.iter().any(|t| t.is_loan),
        completed_transfers,

        active_negotiations,
        seasons,
        lang: route_params.lang,
        i18n,
    })
}

fn get_first_team_slug(simulator_data: &core::SimulatorData, country: &core::Country, club_id: u32) -> String {
    // Try local country first (common case)
    country
        .clubs
        .iter()
        .find(|c| c.id == club_id)
        .and_then(|c| c.teams.teams.first())
        .map(|t| t.slug.clone())
        .or_else(|| {
            // Fall back to global lookup for cross-country transfers
            simulator_data.club(club_id)
                .and_then(|c| c.teams.teams.first())
                .map(|t| t.slug.clone())
        })
        .unwrap_or_default()
}
