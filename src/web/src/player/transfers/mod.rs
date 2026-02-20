pub mod routes;

use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::transfers::{
    NegotiationPhase, NegotiationStatus, TransferListingStatus, TransferListingType,
};
use core::utils::FormattingUtils;
use core::{PlayerStatusType, SimulatorData};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct PlayerTransfersRequest {
    pub lang: String,
    pub player_id: u32,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "player/transfers/index.html")]
pub struct PlayerTransfersTemplate {
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
    pub player_id: u32,
    pub transfer_status: PlayerTransferStatusDto,
    pub listing: Option<PlayerListingDto>,
    pub negotiations: Vec<PlayerNegotiationDto>,
    pub completed: Vec<PlayerCompletedTransferDto>,
}

pub struct PlayerTransferStatusDto {
    pub value: String,
    pub asking_price: String,
    pub is_transfer_listed: bool,
    pub is_loan_listed: bool,
    pub wants_free_transfer: bool,
    pub has_requested_transfer: bool,
    pub has_agreed_transfer: bool,
    pub is_wanted: bool,
    pub has_bid_accepted: bool,
    pub has_enquiry: bool,
    pub status_keys: Vec<String>,
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

pub struct PlayerCompletedTransferDto {
    pub from_club_name: String,
    pub from_club_slug: String,
    pub to_club_name: String,
    pub to_club_slug: String,
    pub fee: String,
    pub date: String,
    pub transfer_type_key: String,
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

    let neighbor_teams: Vec<(String, String)> =
        get_neighbor_teams(team.club_id, simulator_data, &i18n)?;
    let neighbor_refs: Vec<(&str, &str)> = neighbor_teams
        .iter()
        .map(|(n, s)| (n.as_str(), s.as_str()))
        .collect();

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

    let transfer_status = PlayerTransferStatusDto {
        value: FormattingUtils::format_money(player.value(now)),
        asking_price: player
            .contract
            .as_ref()
            .filter(|_| transfer_related.iter().any(|s| *s == PlayerStatusType::Lst))
            .map(|_| FormattingUtils::format_money(player.value(now) * 1.2))
            .unwrap_or_default(),
        is_transfer_listed: transfer_related.contains(&PlayerStatusType::Lst),
        is_loan_listed: transfer_related.contains(&PlayerStatusType::Loa),
        wants_free_transfer: transfer_related.contains(&PlayerStatusType::Frt),
        has_requested_transfer: transfer_related.contains(&PlayerStatusType::Req),
        has_agreed_transfer: transfer_related.contains(&PlayerStatusType::Trn),
        is_wanted: transfer_related.contains(&PlayerStatusType::Wnt),
        has_bid_accepted: transfer_related.contains(&PlayerStatusType::Bid),
        has_enquiry: transfer_related.contains(&PlayerStatusType::Enq),
        status_keys: transfer_related
            .iter()
            .map(|s| status_type_to_i18n_key(s).to_string())
            .collect(),
    };

    // Get transfer listing for this player
    let country = simulator_data.country_by_club(team.club_id);

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

    // Get completed transfers for this player
    let completed: Vec<PlayerCompletedTransferDto> = country
        .map(|c| {
            c.transfer_market
                .transfer_history
                .iter()
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

                    PlayerCompletedTransferDto {
                        from_club_name: t.from_team_name.clone(),
                        from_club_slug: from_slug,
                        to_club_name: t.to_team_name.clone(),
                        to_club_slug: to_slug,
                        fee: if t.fee.amount > 0.0 {
                            FormattingUtils::format_money(t.fee.amount)
                        } else {
                            String::new()
                        },
                        date: t.transfer_date.format("%d.%m.%Y").to_string(),
                        transfer_type_key: transfer_type_key.to_string(),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    let title = format!(
        "{} {}",
        player.full_name.first_name, player.full_name.last_name
    );

    Ok(PlayerTransfersTemplate {
        css_version: crate::common::default_handler::CSS_VERSION,
        title,
        sub_title_prefix: i18n.t(player.position().as_i18n_key()).to_string(),
        sub_title_suffix: if team.team_type == core::TeamType::Main {
            String::new()
        } else {
            i18n.t(team.team_type.as_i18n_key()).to_string()
        },
        sub_title: team.name.clone(),
        sub_title_link: format!("/{}/teams/{}", &route_params.lang, &team.slug),
        header_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.background.clone())
            .unwrap_or_default(),
        foreground_color: simulator_data
            .club(team.club_id)
            .map(|c| c.colors.foreground.clone())
            .unwrap_or_default(),
        menu_sections: views::player_menu(
            &i18n,
            &route_params.lang,
            &neighbor_refs,
            &team.slug,
            &format!("/{}/teams/{}", &route_params.lang, &team.slug),
        ),
        i18n,
        lang: route_params.lang.clone(),
        player_id: route_params.player_id,
        transfer_status,
        listing,
        negotiations,
        completed,
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
        .map(|team| {
            (
                i18n.t(team.team_type.as_i18n_key()).to_string(),
                team.slug.clone(),
                team.reputation.world,
            )
        })
        .collect();

    teams.sort_by(|a, b| b.2.cmp(&a.2));

    Ok(teams
        .into_iter()
        .map(|(name, slug, _)| (name, slug))
        .collect())
}
