pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::common::slug::{PlayerPage, resolve_player_page};
use crate::player::events::PlayerEventsCounter;
use crate::player::newspaper::PlayerNewsCounter;
use crate::views::{self, MenuSection};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::transfers::reason::TransferReason;
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
    pub cpu_brand: &'static str,
    pub cores_count: usize,
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
    pub events_count: usize,
    pub interested_clubs_count: usize,
    pub awards_count: u32,
    pub news_count: usize,
    pub transfer_status: PlayerTransferStatusDto,
    pub listing: Option<PlayerListingDto>,
    pub interested_clubs: Vec<PlayerInterestedClubDto>,
    pub monitoring: Vec<PlayerMonitoringDto>,
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

/// Scout-monitoring summary for the player's transfers page. One row
/// per (scout, club) actively watching this player. Replaces the bare
/// "interested clubs" list with something that names the scout, shows
/// observation count, and surfaces meeting status.
pub struct PlayerMonitoringDto {
    pub club_name: String,
    pub club_slug: String,
    pub scout_name: String,
    pub scout_id: u32,
    /// i18n key — "monitoring_status_active", "..._report_ready", etc.
    pub status_key: String,
    pub last_observed: String,
    /// 0..100 percentage for the UI bar.
    pub confidence_pct: u8,
    pub times_watched: u16,
    pub matches_watched: u16,
}

pub struct PlayerCompletedTransferDto {
    pub from_club_name: String,
    pub from_club_slug: String,
    pub to_club_name: String,
    pub to_club_slug: String,
    pub fee: String,
    pub date: String,
    pub transfer_type_key: String,
    /// Why the club made the move, already localised.
    pub reason: String,
    /// The scout verdict behind the signing, already localised. Empty
    /// when no report backed the deal.
    pub scout: String,
}

/// Renders a core [`TransferReason`] into the two lines a history card
/// shows. The simulator stores the motive as an i18n key and the scout
/// verdict as the raw numbers the report carried, so both are phrased
/// here — in the reader's language — instead of arriving as English.
struct TransferReasonView;

impl TransferReasonView {
    fn motive(i18n: &I18n, reason: &TransferReason) -> String {
        if reason.key.is_empty() {
            return String::new();
        }
        let motive = i18n.t(&reason.key);
        if reason.rival {
            format!("{} {}", motive, i18n.t("signing_reason_rival_suffix"))
        } else {
            motive.to_string()
        }
    }

    fn scout(i18n: &I18n, reason: &TransferReason) -> String {
        let Some(verdict) = reason.scout.as_ref() else {
            return String::new();
        };

        i18n.t("scout_verdict_line")
            .replace(
                "{recommendation}",
                i18n.t(verdict.recommendation.as_i18n_key()),
            )
            .replace("{ability}", i18n.t(verdict.ability_band().as_i18n_key()))
            .replace(
                "{potential}",
                i18n.t(verdict.potential_band().as_i18n_key()),
            )
            .replace("{confidence}", &verdict.confidence_pct().to_string())
    }
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
        PlayerPage::Found {
            player,
            team,
            canonical_slug,
        } => (player, team, canonical_slug),
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
    let league_refs: Vec<(&str, &str)> = country_leagues
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

    let league_rep = team_opt
        .and_then(|t| t.league_id)
        .and_then(|lid| simulator_data.league(lid))
        .map(|l| l.reputation)
        .unwrap_or(0);
    let club_rep = team_opt
        .map(|t| t.reputation.market_value_score())
        .unwrap_or(0);

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
        reason: player
            .decision_history
            .items
            .last()
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
                        buying_club_name: buying_team.map(|t| t.name.clone()).unwrap_or_default(),
                        buying_club_slug: buying_team.map(|t| t.slug.clone()).unwrap_or_default(),
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

    // Detailed monitoring rows: who is watching the player, with what
    // status, how often, and how confident the scout is.
    let monitoring_rows = simulator_data.player_monitoring_details(player.id);
    let monitoring: Vec<PlayerMonitoringDto> = monitoring_rows
        .into_iter()
        .map(|row| PlayerMonitoringDto {
            club_name: row.club_name,
            club_slug: row.team_slug,
            scout_name: row.scout_name.unwrap_or_else(|| {
                // Fall back to a localized "recruitment department" label
                // when no specific scout is named on the row.
                i18n.t("recruitment_department").to_string()
            }),
            scout_id: row.scout_staff_id.unwrap_or(0),
            status_key: format!("monitoring_status_{}", row.status),
            last_observed: row
                .last_observed
                .map(|d| d.format("%d.%m.%Y").to_string())
                .unwrap_or_default(),
            confidence_pct: (row.confidence * 100.0).round().clamp(0.0, 100.0) as u8,
            times_watched: row.times_watched,
            matches_watched: row.matches_watched,
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

                (
                    t.transfer_date,
                    PlayerCompletedTransferDto {
                        from_club_name: t.from_team_name.clone(),
                        from_club_slug: from_slug,
                        to_club_name: t.to_team_name.clone(),
                        to_club_slug: to_slug,
                        fee: if t.fee.amount > 0.0 {
                            FormattingUtils::format_money(t.fee.amount)
                        } else {
                            i18n.t("fee_free").to_string()
                        },
                        date: t.transfer_date.format("%d.%m.%Y").to_string(),
                        transfer_type_key: transfer_type_key.to_string(),
                        reason: TransferReasonView::motive(&i18n, &t.reason),
                        scout: TransferReasonView::scout(&i18n, &t.reason),
                    },
                )
            })
            .collect();
        transfers.sort_by(|a, b| b.0.cmp(&a.0));
        // Deduplicate cross-country transfers (stored in both countries' histories)
        transfers.dedup_by(|a, b| {
            a.0 == b.0
                && a.1.from_club_name == b.1.from_club_name
                && a.1.to_club_name == b.1.to_club_name
        });
        transfers.into_iter().map(|(_, dto)| dto).collect()
    };

    let title = format!(
        "{} {}",
        player.full_name.display_first_name(),
        player.full_name.display_last_name()
    );

    Ok(PlayerTransfersTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
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
            views::team_menu(&mp, &neighbor_refs, &league_refs)
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
        is_force_match_selection: player.is_force_match_selection,
        is_on_watchlist: simulator_data.watchlist.contains(&player.id),
        events_count: PlayerEventsCounter::count(player),
        interested_clubs_count: interested_clubs.len(),
        awards_count: player.awards_count.total(),
        news_count: PlayerNewsCounter::count(simulator_data, player),
        transfer_status,
        listing,
        interested_clubs,
        monitoring,
        negotiations,
        completed,
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

#[cfg(test)]
mod tests {
    use super::*;
    use core::club::player::calculators::FreeAgentReleaseReason;
    use core::transfers::pipeline::{ScoutingRecommendation, TransferNeedReason};
    use core::transfers::reason::{AbilityBand, ScoutVerdict};
    use std::collections::HashMap;

    fn en_map() -> HashMap<String, String> {
        serde_json::from_str(include_str!("../../../assets/i18n/en.json"))
            .expect("en.json is not valid JSON")
    }

    /// Every motive the simulator can stamp on a completed transfer.
    /// The history row renders `i18n.t(reason.key)`, which returns the
    /// key itself when the bundle has no copy for it — a missing entry
    /// shows up as raw snake_case on the page instead of failing.
    const FIXED_REASON_KEYS: &[&str] = &[
        "signing_reason_loan",
        "signing_reason_transfer",
        "signing_reason_loan_opportunistic_upgrade",
        "signing_reason_loan_midseason_reinforcement",
        "signing_reason_loan_development_approach",
        "signing_reason_loan_broadcast",
        "signing_reason_loan_foreign_prospect",
        "signing_reason_listing_broadcast",
        "signing_reason_academy_graduation",
        "signing_reason_academy_emergency_callup",
        "signing_reason_manual",
        "signing_reason_rival_suffix",
        "scout_verdict_line",
        "fee_free",
        "pre_contract",
        "free_agent_market_clearing",
        "emergency_squad_fill_gk",
        "emergency_squad_fill_def",
        "emergency_squad_fill_mid",
        "emergency_squad_fill_fwd",
        "emergency_squad_fill_depth",
    ];

    const NEED_REASONS: &[TransferNeedReason] = &[
        TransferNeedReason::FormationGap,
        TransferNeedReason::QualityUpgrade,
        TransferNeedReason::DepthCover,
        TransferNeedReason::SuccessionPlanning,
        TransferNeedReason::DevelopmentSigning,
        TransferNeedReason::StaffRecommendation,
        TransferNeedReason::LoanToFillSquad,
        TransferNeedReason::ExperiencedHead,
        TransferNeedReason::SquadPadding,
        TransferNeedReason::CheapReinforcement,
        TransferNeedReason::InjuryCoverLoan,
        TransferNeedReason::OpportunisticLoanUpgrade,
    ];

    const RELEASE_REASONS: &[FreeAgentReleaseReason] = &[
        FreeAgentReleaseReason::ContractExpired,
        FreeAgentReleaseReason::MutualTermination,
        FreeAgentReleaseReason::SurplusFreeRelease,
        FreeAgentReleaseReason::FailedRenewalRelease,
        FreeAgentReleaseReason::AcademyAgedOut,
        FreeAgentReleaseReason::Under16Release,
        FreeAgentReleaseReason::UnsoldListingExit,
    ];

    const ABILITY_BANDS: &[AbilityBand] = &[
        AbilityBand::VeryPoor,
        AbilityBand::Poor,
        AbilityBand::BelowAverage,
        AbilityBand::Average,
        AbilityBand::Decent,
        AbilityBand::Good,
        AbilityBand::VeryGood,
        AbilityBand::Excellent,
        AbilityBand::WorldClass,
        AbilityBand::Unknown,
    ];

    #[test]
    fn every_transfer_reason_key_has_english_copy() {
        let map = en_map();
        let mut keys: Vec<String> = FIXED_REASON_KEYS.iter().map(|k| k.to_string()).collect();
        keys.extend(
            NEED_REASONS
                .iter()
                .map(|r| r.as_signing_reason_key().to_string()),
        );
        keys.extend(
            RELEASE_REASONS
                .iter()
                .map(|r| r.history_reason().to_string()),
        );
        keys.extend(ABILITY_BANDS.iter().map(|b| b.as_i18n_key().to_string()));
        keys.extend(
            [
                ScoutingRecommendation::StrongBuy,
                ScoutingRecommendation::Buy,
                ScoutingRecommendation::Consider,
                ScoutingRecommendation::Pass,
            ]
            .iter()
            .map(|r| r.as_i18n_key().to_string()),
        );

        let missing: Vec<&String> = keys.iter().filter(|k| !map.contains_key(*k)).collect();
        assert!(
            missing.is_empty(),
            "en.json is missing {} transfer-reason key(s): {:?}",
            missing.len(),
            missing
        );
    }

    #[test]
    fn a_scouted_signing_renders_both_lines_localised() {
        let i18n = I18n::for_test(en_map());
        let reason =
            TransferReason::key(TransferNeedReason::DevelopmentSigning.as_signing_reason_key())
                .with_scout(Some(ScoutVerdict {
                    recommendation: ScoutingRecommendation::Buy,
                    assessed_ability: 95,
                    assessed_potential: 110,
                    confidence: 0.4,
                }));

        let motive = TransferReasonView::motive(&i18n, &reason);
        let scout = TransferReasonView::scout(&i18n, &reason);

        assert_eq!(
            motive,
            "Development signing — young prospect with high potential"
        );
        assert_eq!(
            scout,
            "Scout: Buy (ability: Average, potential: Decent, confidence: 40%)"
        );
        assert!(
            !scout.contains('{'),
            "every placeholder must be substituted: {scout}"
        );
    }

    #[test]
    fn a_rival_raid_is_marked_and_a_bare_reason_renders_nothing() {
        let i18n = I18n::for_test(en_map());
        let raid = TransferReason::key("signing_reason_transfer").as_rival();

        assert_eq!(
            TransferReasonView::motive(&i18n, &raid),
            "Transfer signing (rival raid)"
        );
        assert!(TransferReasonView::scout(&i18n, &raid).is_empty());
        assert!(TransferReasonView::motive(&i18n, &TransferReason::default()).is_empty());
    }
}
