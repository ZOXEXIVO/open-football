pub mod routes;

use crate::common::default_handler::{CSS_VERSION, COMPUTER_NAME};
use crate::common::slug::{resolve_player_page, PlayerPage};
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::transfers::{
    NegotiationPhase, NegotiationStatus, TransferListingStatus, TransferListingType,
};
use core::utils::FormattingUtils;
use core::{PlayerStatusType, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerTransfersRequest {
    pub lang: String,
    pub player_slug: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/transfers/index.html")]
pub struct PlayerTransfersTemplate {
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
    pub is_on_watchlist: bool,
    pub transfer_status: PlayerTransferStatusDto,
    pub listing: Option<PlayerListingDto>,
    pub interested_clubs: Vec<PlayerInterestedClubDto>,
    pub negotiations: Vec<PlayerNegotiationDto>,
    pub completed: Vec<PlayerCompletedTransferDto>,
}

pub struct PlayerTransferStatusDto {
    pub value: String,
    pub asking_price: String,
    pub status_keys: Vec<String>,
    pub reason: String,
}

pub struct PlayerListingDto {
    pub listing_type_key: String,
    pub asking_price: String,
    pub listed_date: String,
    pub status_key: String,
}

pub struct PlayerNegotiationDto {
    pub buying_club_name: String,
    pub buying_club_slug: String,
    pub offer_amount: String,
    pub phase_key: String,
    pub status_key: String,
    pub started_date: String,
    pub is_loan: bool,
}

pub struct PlayerInterestedClubDto {
    pub club_name: String,
    pub club_slug: String,
}

pub struct PlayerCompletedTransferDto {
    pub from_club_name: String,
    pub from_club_slug: String,
    pub to_club_name: String,
    pub to_club_slug: String,
    pub fee: String,
    pub date: String,
    pub transfer_type_key: String,
    pub reason: String,
}

fn status_type_to_i18n_key(status: &PlayerStatusType) -> &'static str {
    match status {
        PlayerStatusType::Lst => "player_status_listed",
        PlayerStatusType::Loa => "player_status_loan_listed",
        PlayerStatusType::Frt => "player_status_free_transfer",
        PlayerStatusType::Req => "player_status_requested",
        PlayerStatusType::Trn => "player_status_agreed",
        PlayerStatusType::Wnt => "player_status_wanted",
        PlayerStatusType::Bid => "player_status_bid_accepted",
        PlayerStatusType::Enq => "player_status_enquiry",
        PlayerStatusType::Unh => "player_status_unhappy",
        _ => "player_status_none",
    }
}

fn negotiation_phase_to_key(phase: &NegotiationPhase) -> &'static str {
    match phase {
        NegotiationPhase::InitialApproach { .. } => "neg_phase_approach",
        NegotiationPhase::ClubNegotiation { .. } => "neg_phase_club",
        NegotiationPhase::PersonalTerms { .. } => "neg_phase_personal",
        NegotiationPhase::MedicalAndFinalization { .. } => "neg_phase_medical",
    }
}

fn negotiation_status_to_key(status: &NegotiationStatus) -> &'static str {
    match status {
        NegotiationStatus::Pending => "neg_status_pending",
        NegotiationStatus::Accepted => "neg_status_accepted",
        NegotiationStatus::Rejected => "neg_status_rejected",
        NegotiationStatus::Countered => "neg_status_countered",
        NegotiationStatus::Expired => "neg_status_expired",
    }
}

fn listing_type_to_key(listing_type: &TransferListingType) -> &'static str {
    match listing_type {
        TransferListingType::Transfer => "listing_type_transfer",
        TransferListingType::Loan => "listing_type_loan",
        TransferListingType::EndOfContract => "listing_type_free",
    }
}

fn listing_status_to_key(status: &TransferListingStatus) -> &'static str {
    match status {
        TransferListingStatus::Available => "listing_status_available",
        TransferListingStatus::InNegotiation => "listing_status_negotiating",
        TransferListingStatus::Completed => "listing_status_completed",
        TransferListingStatus::Cancelled => "listing_status_cancelled",
    }
}

pub async fn player_transfers_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<PlayerTransfersRequest>,
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
        "/transfers",
    )? {
        PlayerPage::Found { player, team, canonical_slug } => (player, team, canonical_slug),
        PlayerPage::Redirect(r) => return Ok(r),
    };

    let (neighbor_teams, country_leagues) = if let Some(team) = team_opt {
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?
    } else {
        (Vec::new(), Vec::new())
    };
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();
    let league_refs: Vec<(&str, &str)> = country_leagues.iter().map(|(n, s)| (n.as_str(), s.as_str())).collect();

    let now = simulator_data.date.date();

    // Build transfer status from player statuses
    let statuses = player.statuses.get();
    let transfer_related: Vec<PlayerStatusType> = statuses
        .iter()
        .filter(|s| {
            matches!(
                s,
                PlayerStatusType::Lst
                    | PlayerStatusType::Loa
                    | PlayerStatusType::Frt
                    | PlayerStatusType::Req
                    | PlayerStatusType::Trn
                    | PlayerStatusType::Wnt
                    | PlayerStatusType::Bid
                    | PlayerStatusType::Enq
                    | PlayerStatusType::Unh
            )
        })
        .copied()
        .collect();

    let league_rep = team_opt.and_then(|t| t.league_id).and_then(|lid| simulator_data.league(lid)).map(|l| l.reputation).unwrap_or(0);
    let club_rep = team_opt.map(|t| t.reputation.world).unwrap_or(0);

    let transfer_status = PlayerTransferStatusDto {
        value: FormattingUtils::format_money(player.value(now, league_rep, club_rep)),
        asking_price: player
            .contract
            .as_ref()
            .filter(|_| transfer_related.iter().any(|s| *s == PlayerStatusType::Lst))
            .map(|_| FormattingUtils::format_money(player.value(now, league_rep, club_rep) * 1.2))
            .unwrap_or_default(),
        status_keys: transfer_related
            .iter()
            .map(|s| status_type_to_i18n_key(s).to_string())
            .collect(),
        reason: player.decision_history.items.last()
            .map(|d| i18n.t(&d.decision).to_string())
            .unwrap_or_default(),
    };

    // Get transfer listing for this player
    let country = team_opt.and_then(|t| simulator_data.country_by_club(t.club_id));

    let listing = country.and_then(|c| {
        c.transfer_market
            .get_listing_by_player(player.id)
            .map(|l| PlayerListingDto {
                listing_type_key: listing_type_to_key(&l.listing_type).to_string(),
                asking_price: FormattingUtils::format_money(l.asking_price.amount),
                listed_date: l.listed_date.format("%d.%m.%Y").to_string(),
                status_key: listing_status_to_key(&l.status).to_string(),
            })
    });

    // Get active negotiations for this player
    let negotiations: Vec<PlayerNegotiationDto> = country
        .map(|c| {
            c.transfer_market
                .negotiations
                .values()
                .filter(|n| {
                    n.player_id == player.id
                        && (n.status == NegotiationStatus::Pending
                        || n.status == NegotiationStatus::Countered)
                })
                .map(|n| {
                    let buying_club = simulator_data.club(n.buying_club_id);
                    let buying_team = buying_club.and_then(|c| c.teams.teams.first());

                    PlayerNegotiationDto {
                        buying_club_name: buying_team
                            .map(|t| t.name.clone())
                            .unwrap_or_default(),
                        buying_club_slug: buying_team
                            .map(|t| t.slug.clone())
                            .unwrap_or_default(),
                        offer_amount: FormattingUtils::format_money(
                            n.current_offer.base_fee.amount,
                        ),
                        phase_key: negotiation_phase_to_key(&n.phase).to_string(),
                        status_key: negotiation_status_to_key(&n.status).to_string(),
                        started_date: n.created_date.format("%d.%m.%Y").to_string(),
                        is_loan: n.is_loan,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Get clubs interested in this player (scouting/shortlisted)
    let interested_clubs: Vec<PlayerInterestedClubDto> = simulator_data
        .clubs_interested_in_player(player.id)
        .into_iter()
        .map(|(_club_id, club_name, team_slug)| PlayerInterestedClubDto {
            club_name,
            club_slug: team_slug,
        })
        .collect();

    // Get completed transfers for this player (all seasons, across all countries)
    let completed: Vec<PlayerCompletedTransferDto> = {
        let mut transfers: Vec<_> = simulator_data
            .continents
            .iter()
            .flat_map(|cont| &cont.countries)
            .flat_map(|c| &c.transfer_market.transfer_history)
            .filter(|t| t.player_id == player.id)
            .map(|t| {
                let from_slug = simulator_data
                    .club(t.from_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|team| team.slug.clone())
                    .unwrap_or_default();
                let to_slug = simulator_data
                    .club(t.to_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|team| team.slug.clone())
                    .unwrap_or_default();

                let transfer_type_key = match &t.transfer_type {
                    core::transfers::TransferType::Permanent => "transfer_type_permanent",
                    core::transfers::TransferType::Loan(_) => "transfer_type_loan",
                    core::transfers::TransferType::Free => "transfer_type_free",
                };

                (t.transfer_date, PlayerCompletedTransferDto {
                    from_club_name: t.from_team_name.clone(),
                    from_club_slug: from_slug,
                    to_club_name: t.to_team_name.clone(),
                    to_club_slug: to_slug,
                    fee: if t.fee.amount > 0.0 {
                        FormattingUtils::format_money(t.fee.amount)
                    } else {
                        "Free".to_string()
                    },
                    date: t.transfer_date.format("%d.%m.%Y").to_string(),
                    transfer_type_key: transfer_type_key.to_string(),
                    reason: i18n.t(&t.reason).to_string(),
                })
            })
            .collect();
        transfers.sort_by(|a, b| b.0.cmp(&a.0));
        // Deduplicate cross-country transfers (stored in both countries' histories)
        transfers.dedup_by(|a, b| a.0 == b.0 && a.1.from_club_name == b.1.from_club_name && a.1.to_club_name == b.1.to_club_name);
        transfers.into_iter().map(|(_, dto)| dto).collect()
    };

    let title = format!(
        "{} {}",
        player.full_name.display_first_name(), player.full_name.display_last_name()
    );

    Ok(PlayerTransfersTemplate {
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
        sub_title_link: team_opt.map(|t| format!("/{}/teams/{}", &route_params.lang, &t.slug)).unwrap_or_default(),
        sub_title_country_code: String::new(),
        header_color: team_opt.and_then(|t| simulator_data.club(t.club_id).map(|c| c.colors.background.clone())).unwrap_or_else(|| "#808080".to_string()),
        foreground_color: team_opt.and_then(|t| simulator_data.club(t.club_id).map(|c| c.colors.foreground.clone())).unwrap_or_else(|| "#ffffff".to_string()),
        menu_sections: if let Some(team) = team_opt {
            let (cn, cs) = views::club_country_info(simulator_data, team.club_id);
            let current_path = format!("/{}/teams/{}", &route_params.lang, &team.slug);
            let mp = views::MenuParams { i18n: &i18n, lang: &route_params.lang, current_path: &current_path, country_name: cn, country_slug: cs };
            views::team_menu(&mp, &neighbor_refs, &team.slug, &league_refs, team.team_type == core::TeamType::Main)
        } else {
            Vec::new()
        },
        i18n,
        lang: route_params.lang.clone(),
        active_tab: "transfers",
        player_id: player.id,
        player_slug: canonical,
        club_id: team_opt.map(|t| t.club_id).unwrap_or(0),
        is_on_loan: player.is_on_loan(),
        is_injured: player.player_attributes.is_injured,
        is_unhappy: player.statuses.get().contains(&PlayerStatusType::Unh),
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        transfer_status,
        listing,
        interested_clubs,
        negotiations,
        completed,
    }.into_response())
}

fn get_neighbor_teams(
    club_id: u32,
    data: &SimulatorData,
    i18n: &I18n,
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
