pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::transfers::TransferType;
use core::utils::FormattingUtils;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct LeagueTransfersRequest {
    lang: String,
    league_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "leagues/transfers/index.html")]
pub struct LeagueTransfersTemplate {
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
    pub league_slug: String,
    pub completed_transfers: Vec<CompletedTransferItem>,
    pub current_listings: Vec<ListingItem>,
    pub active_negotiations: Vec<NegotiationItem>,
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

#[allow(dead_code)]
pub struct ListingItem {
    pub player_id: u32,
    pub player_name: String,
    pub team_name: String,
    pub team_slug: String,
    pub asking_price: String,
    pub status: String,
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

    // Get club IDs that belong to this league's teams
    let league_club_ids: Vec<u32> = league
        .table
        .get()
        .iter()
        .filter_map(|row| simulator_data.team(row.team_id).map(|t| t.club_id))
        .collect();

    // Completed transfers involving league clubs
    let completed_transfers: Vec<CompletedTransferItem> = country
        .transfer_market
        .transfer_history
        .iter()
        .filter(|t| {
            league_club_ids.contains(&t.from_club_id) || league_club_ids.contains(&t.to_club_id)
        })
        .map(|t| {
            let from_team_slug = get_first_team_slug(country, t.from_club_id);
            let to_team_slug = get_first_team_slug(country, t.to_club_id);
            CompletedTransferItem {
                player_id: t.player_id,
                player_name: t.player_name.clone(),
                from_team: t.from_team_name.clone(),
                from_team_slug,
                to_team: t.to_team_name.clone(),
                to_team_slug,
                fee: FormattingUtils::format_money(t.fee.amount),
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

            // Find the player name
            let player_name = selling_club
                .teams
                .teams
                .iter()
                .flat_map(|t| &t.players.players)
                .find(|p| p.id == n.player_id)
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

    // Collect player IDs that have active negotiations
    let negotiating_player_ids: Vec<u32> = active_negotiations
        .iter()
        .map(|n| n.player_id)
        .collect();

    // Current listings from league clubs, excluding those in active negotiations
    let current_listings: Vec<ListingItem> = country
        .transfer_market
        .listings
        .iter()
        .filter(|l| league_club_ids.contains(&l.club_id))
        .filter(|l| !negotiating_player_ids.contains(&l.player_id))
        .filter_map(|l| {
            let club = country.clubs.iter().find(|c| c.id == l.club_id)?;
            let player = club
                .teams
                .teams
                .iter()
                .flat_map(|t| &t.players.players)
                .find(|p| p.id == l.player_id)?;

            let team_slug = club.teams.teams.first()
                .map(|t| t.slug.clone())
                .unwrap_or_default();
            Some(ListingItem {
                player_id: player.id,
                player_name: player.full_name.to_string(),
                team_name: club.name.clone(),
                team_slug,
                asking_price: FormattingUtils::format_money(l.asking_price.amount),
                status: format!("{:?}", l.status),
            })
        })
        .collect();

    Ok(LeagueTransfersTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title: format!("{} - Transfers", league.name),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: country.name.clone(),
        sub_title_link: format!("/{}/countries/{}", &route_params.lang, &country.slug),
        sub_title_country_code: country.code.clone(),
        header_color: country.background_color.clone(),
        foreground_color: country.foreground_color.clone(),
        menu_sections: {
            let mut cl: Vec<(u32, &str, &str)> = country.leagues.leagues.iter().map(|l| (l.id, l.name.as_str(), l.slug.as_str())).collect();
            cl.sort_by_key(|(id, _, _)| *id);
            let cl_refs: Vec<(&str, &str)> = cl.iter().map(|(_, n, s)| (*n, *s)).collect();
            views::league_menu(&i18n, &route_params.lang, &country.name, &country.slug, &league.slug, &format!("/{}/leagues/{}/transfers", &route_params.lang, &league.slug), &cl_refs)
        },
        league_slug: league.slug.clone(),
        completed_transfers,
        current_listings,
        active_negotiations,
        lang: route_params.lang,
        i18n,
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
