pub mod routes;

use crate::leagues::address::LeagueAddress;
use crate::player::{PlayerPageChrome, PlayerPageColors};
use crate::shared::slug::{PlayerPage, PlayerSlug};
use crate::views::{ClubCountryFlag, MenuSection, Menus, SubTitleFlag};
use crate::{ApiError, ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use core::transfers::deal::reason::TransferReason;
use core::transfers::{
    NegotiationPhase, NegotiationStatus, TransferListingStatus, TransferListingType,
};
use core::utils::FormattingUtils;
use core::{Player, PlayerStatusType, SimulatorData, Team};
use serde::Deserialize;
use std::cmp::Reverse;

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
    pub sub_title_flag: Option<SubTitleFlag>,
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
    pub watch_key: String,
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
    pub country_flag: Option<ClubCountryFlag>,
}

/// A scout observation or recruitment summary on the player's transfers page.
pub struct PlayerMonitoringDto {
    pub club_name: String,
    pub club_slug: String,
    pub country: Option<ClubCountryFlag>,
    pub league: Option<MonitoringLeague>,
    world_reputation: Option<u16>,
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

pub struct MonitoringLeague {
    pub name: String,
    pub path: Option<String>,
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

/// Why the club put him on the market, beside the listing status: the
/// reason recorded with the decision that listed him — never whichever
/// row the register happened to write last — and nothing when he is not
/// listed at all.
struct ListingReasonView;

impl ListingReasonView {
    fn of(player: &Player, statuses: &[PlayerStatusType], i18n: &I18n) -> String {
        let listed = statuses.iter().any(|s| {
            matches!(
                s,
                PlayerStatusType::Lst | PlayerStatusType::Loa | PlayerStatusType::Frt
            )
        });
        if !listed {
            return String::new();
        }
        player
            .decision_history
            .latest_listing()
            .map(|d| i18n.t(&d.decision).to_string())
            .unwrap_or_default()
    }
}

struct TransferLabels;

impl TransferLabels {
    fn status(status: &PlayerStatusType) -> &'static str {
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

    fn negotiation_phase(phase: &NegotiationPhase) -> &'static str {
        match phase {
            NegotiationPhase::InitialApproach { .. } => "neg_phase_approach",
            NegotiationPhase::ClubNegotiation { .. } => "neg_phase_club",
            NegotiationPhase::PersonalTerms { .. } => "neg_phase_personal",
            NegotiationPhase::MedicalAndFinalization { .. } => "neg_phase_medical",
        }
    }

    fn listing_type(listing_type: &TransferListingType) -> &'static str {
        match listing_type {
            TransferListingType::Transfer => "listing_type_transfer",
            TransferListingType::Loan => "listing_type_loan",
            TransferListingType::EndOfContract => "listing_type_free",
        }
    }

    fn listing_status(status: &TransferListingStatus) -> &'static str {
        match status {
            TransferListingStatus::Available => "listing_status_available",
            TransferListingStatus::InNegotiation => "listing_status_negotiating",
            TransferListingStatus::Completed => "listing_status_completed",
            TransferListingStatus::Cancelled => "listing_status_cancelled",
        }
    }
}

pub struct PlayerTransfersPage;

impl PlayerTransfersPage {
    pub async fn action(
        State(state): State<GameAppData>,
        Path(route_params): Path<PlayerTransfersRequest>,
    ) -> ApiResult<Response> {
        let i18n = state.i18n.for_lang(&route_params.lang);
        let guard = state.data.read().await;

        let simulator_data = guard
            .as_ref()
            .ok_or_else(|| ApiError::InternalError("Simulator data not loaded".to_string()))?;

        let (player, team_opt, canonical) = match PlayerSlug::resolve_page(
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

        let now = simulator_data.date.date();
        let view = TransfersView::build(player, team_opt, simulator_data, &i18n, now);
        let chrome = PlayerPageChrome::build(
            simulator_data,
            i18n,
            player,
            team_opt,
            &route_params.lang,
            &canonical,
            "transfers",
            PlayerPageColors::Grey,
        )?;

        Ok(PlayerTransfersTemplate {
            css_version: chrome.css_version,
            computer_name: chrome.computer_name,
            cpu_brand: chrome.cpu_brand,
            cores_count: chrome.cores_count,
            title: chrome.title,
            sub_title_prefix: chrome.sub_title_prefix,
            sub_title_suffix: chrome.sub_title_suffix,
            sub_title: chrome.sub_title,
            sub_title_link: chrome.sub_title_link,
            sub_title_flag: chrome.sub_title_flag,
            header_color: chrome.header_color,
            foreground_color: chrome.foreground_color,
            menu_sections: chrome.menu_sections,
            i18n: chrome.i18n,
            lang: chrome.lang,
            active_tab: chrome.active_tab,
            player_id: chrome.player_id,
            player_slug: chrome.player_slug,
            club_id: chrome.club_id,
            is_on_loan: chrome.is_on_loan,
            is_injured: chrome.is_injured,
            is_unhappy: chrome.is_unhappy,
            is_force_match_selection: chrome.is_force_match_selection,
            watch_key: chrome.watch_key,
            events_count: chrome.events_count,
            interested_clubs_count: chrome.interested_clubs_count,
            awards_count: chrome.awards_count,
            news_count: chrome.news_count,
            transfer_status: view.transfer_status,
            listing: view.listing,
            interested_clubs: view.interested_clubs,
            monitoring: view.monitoring,
            negotiations: view.negotiations,
            completed: view.completed,
        }
        .into_response())
    }
}

struct TransfersView {
    transfer_status: PlayerTransferStatusDto,
    listing: Option<PlayerListingDto>,
    interested_clubs: Vec<PlayerInterestedClubDto>,
    monitoring: Vec<PlayerMonitoringDto>,
    negotiations: Vec<PlayerNegotiationDto>,
    completed: Vec<PlayerCompletedTransferDto>,
}

impl TransfersView {
    fn build(
        player: &Player,
        team_opt: Option<&Team>,
        data: &SimulatorData,
        i18n: &I18n,
        now: chrono::NaiveDate,
    ) -> Self {
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
            .and_then(|lid| data.league(lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let club_rep = team_opt
            .map(|t| t.reputation.market_value_score())
            .unwrap_or(0);
        let country = team_opt.and_then(|t| data.country_by_club(t.club_id));

        Self {
            transfer_status: Self::transfer_status(
                player,
                &transfer_related,
                now,
                league_rep,
                club_rep,
                i18n,
            ),
            listing: Self::listing(country, player),
            negotiations: Self::negotiations(country, data, player),
            interested_clubs: data
                .clubs_interested_in_player(player.id)
                .into_iter()
                .map(|(club_id, club_name, team_slug)| PlayerInterestedClubDto {
                    club_name,
                    club_slug: team_slug,
                    country_flag: ClubCountryFlag::for_club(data, club_id, i18n),
                })
                .collect(),
            monitoring: Self::monitoring(data, player, i18n),
            completed: Self::completed(data, player, i18n),
        }
    }

    fn transfer_status(
        player: &Player,
        transfer_related: &[PlayerStatusType],
        now: chrono::NaiveDate,
        league_rep: u16,
        club_rep: u16,
        i18n: &I18n,
    ) -> PlayerTransferStatusDto {
        PlayerTransferStatusDto {
            value: FormattingUtils::format_money(player.value(now, league_rep, club_rep)),
            asking_price: player
                .contract
                .as_ref()
                .filter(|_| transfer_related.contains(&PlayerStatusType::Lst))
                .map(|_| {
                    FormattingUtils::format_money(player.value(now, league_rep, club_rep) * 1.2)
                })
                .unwrap_or_default(),
            status_keys: transfer_related
                .iter()
                .map(|s| TransferLabels::status(s).to_string())
                .collect(),
            reason: ListingReasonView::of(player, transfer_related, i18n),
        }
    }

    fn listing(country: Option<&core::Country>, player: &Player) -> Option<PlayerListingDto> {
        country.and_then(|c| {
            c.transfer_market
                .get_listing_by_player(player.id)
                .map(|l| PlayerListingDto {
                    listing_type_key: TransferLabels::listing_type(&l.listing_type).to_string(),
                    asking_price: FormattingUtils::format_money(l.asking_price.amount),
                    listed_date: l.listed_date.format("%d.%m.%Y").to_string(),
                    status_key: TransferLabels::listing_status(&l.status).to_string(),
                })
        })
    }

    fn negotiations(
        country: Option<&core::Country>,
        data: &SimulatorData,
        player: &Player,
    ) -> Vec<PlayerNegotiationDto> {
        country
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
                        let buying_club = data.club(n.buying_club_id);
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
                            phase_key: TransferLabels::negotiation_phase(&n.phase).to_string(),
                            status_key: n.status.as_i18n_key().to_string(),
                            started_date: n.created_date.format("%d.%m.%Y").to_string(),
                            is_loan: n.is_loan,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn monitoring(data: &SimulatorData, player: &Player, i18n: &I18n) -> Vec<PlayerMonitoringDto> {
        let mut rows: Vec<_> = data
            .player_monitoring_details(player.id)
            .into_iter()
            .map(|row| {
                let senior_team = data.club(row.club_id).and_then(|club| club.teams.main());
                let league = senior_team
                    .and_then(|team| team.league_id)
                    .and_then(|id| data.league(id))
                    .filter(|league| !league.friendly && !league.name.is_empty())
                    .map(|league| MonitoringLeague {
                        name: Menus::league_display_name(league, i18n, data),
                        path: data
                            .country(league.country_id)
                            .filter(|country| !country.slug.is_empty() && !league.slug.is_empty())
                            .map(|country| LeagueAddress::new(&country.slug, &league.slug).path()),
                    });

                PlayerMonitoringDto {
                    club_name: row.club_name,
                    club_slug: row.team_slug,
                    country: ClubCountryFlag::for_club(data, row.club_id, i18n),
                    league,
                    world_reputation: senior_team.map(|team| team.reputation.world),
                    scout_name: row
                        .scout_name
                        .unwrap_or_else(|| i18n.t("recruitment_department").to_string()),
                    scout_id: row.scout_staff_id.unwrap_or(0),
                    status_key: format!("monitoring_status_{}", row.status),
                    last_observed: row
                        .last_observed
                        .map(|d| d.format("%d.%m.%Y").to_string())
                        .unwrap_or_default(),
                    confidence_pct: (row.confidence * 100.0).round().clamp(0.0, 100.0) as u8,
                    times_watched: row.times_watched,
                    matches_watched: row.matches_watched,
                }
            })
            .collect();
        rows.sort_by_key(|row| Reverse(row.world_reputation));
        rows
    }

    fn completed(
        data: &SimulatorData,
        player: &Player,
        i18n: &I18n,
    ) -> Vec<PlayerCompletedTransferDto> {
        let mut transfers: Vec<_> = data
            .continents
            .iter()
            .flat_map(|cont| &cont.countries)
            .flat_map(|c| &c.transfer_market.transfer_history)
            .filter(|t| t.player_id == player.id)
            .map(|t| {
                let from_slug = data
                    .club(t.from_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|team| team.slug.clone())
                    .unwrap_or_default();
                let to_slug = data
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
                        from_club_name: if t.from_club_id == 0 {
                            i18n.t("free_agent").to_string()
                        } else {
                            t.from_team_name.clone()
                        },
                        from_club_slug: from_slug,
                        to_club_name: if t.to_club_id == 0 {
                            i18n.t("free_agent").to_string()
                        } else {
                            t.to_team_name.clone()
                        },
                        to_club_slug: to_slug,
                        fee: if t.fee.amount > 0.0 {
                            FormattingUtils::format_money(t.fee.amount)
                        } else {
                            i18n.t("fee_free").to_string()
                        },
                        date: t.transfer_date.format("%d.%m.%Y").to_string(),
                        transfer_type_key: transfer_type_key.to_string(),
                        reason: TransferReasonView::motive(i18n, &t.reason),
                        scout: TransferReasonView::scout(i18n, &t.reason),
                    },
                )
            })
            .collect();
        transfers.sort_by_key(|t| Reverse(t.0));
        // Deduplicate cross-country transfers (stored in both countries' histories)
        transfers.dedup_by(|a, b| {
            a.0 == b.0
                && a.1.from_club_name == b.1.from_club_name
                && a.1.to_club_name == b.1.to_club_name
        });
        transfers.into_iter().map(|(_, dto)| dto).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::transfers::routes::TeamTransfersRoutes;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use chrono::{NaiveDate, NaiveTime};
    use core::club::ClubAcademy;
    use core::club::player::builder::PlayerBuilder;
    use core::club::player::calculators::FreeAgentReleaseReason;
    use core::competitions::global::GlobalCompetitions;
    use core::continent::Continent;
    use core::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use core::shared::fullname::FullName;
    use core::shared::{Currency, CurrencyValue, Location};
    use core::transfers::deal::reason::{AbilityBand, ScoutVerdict};
    use core::transfers::gate::TransferMoveStage;
    use core::transfers::pipeline::{
        FileObservation, FileReading, ScoutingRecommendation, ShortlistCandidate,
        ShortlistCandidateStatus, TransferNeedReason,
    };
    use core::transfers::scouting::recruitment::ScoutMonitoringSource;
    use core::transfers::{CompletedTransfer, TransferType};
    use core::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType, PlayerPositions,
        PlayerSkills, StaffCollection, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use std::collections::HashMap;
    use std::sync::Arc;
    use tower::ServiceExt;

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
        TransferNeedReason::Replacement,
    ];

    const RELEASE_REASONS: &[FreeAgentReleaseReason] = &[
        FreeAgentReleaseReason::ContractExpired,
        FreeAgentReleaseReason::MutualTermination,
        FreeAgentReleaseReason::SurplusFreeRelease,
        FreeAgentReleaseReason::FailedRenewalRelease,
        FreeAgentReleaseReason::AcademyAgedOut,
        FreeAgentReleaseReason::Under16Release,
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

    fn loan_listed_then_pathway_changed() -> Player {
        let mut player = PlayerBuilder::new()
            .id(1)
            .full_name(FullName::new("Seb".to_string(), "Nava".to_string()))
            .birth_date(NaiveDate::from_ymd_opt(2008, 5, 19).unwrap())
            .country_id(1)
            .attributes(PersonAttributes::default())
            .skills(PlayerSkills::default())
            .positions(PlayerPositions {
                positions: vec![PlayerPosition {
                    position: PlayerPositionType::Goalkeeper,
                    level: 18,
                }],
            })
            .player_attributes(PlayerAttributes::default())
            .build()
            .unwrap();
        let day = NaiveDate::from_ymd_opt(2031, 11, 1).unwrap();
        player.decision_history.add(
            day,
            "dec_board_loan_listed".to_string(),
            "dec_reason_development_pathway".to_string(),
            "dec_decided_board".to_string(),
        );
        player.decision_history.add(
            day,
            "dec_pathway_stage_changed".to_string(),
            "pathway_stage_loan_out".to_string(),
            "dec_decided_board".to_string(),
        );
        player
    }

    #[test]
    fn a_pathway_change_after_the_listing_does_not_replace_its_reason() {
        let i18n = I18n::for_test(en_map());
        let player = loan_listed_then_pathway_changed();
        assert_eq!(
            ListingReasonView::of(&player, &[PlayerStatusType::Loa], &i18n),
            i18n.t("dec_reason_development_pathway")
        );
    }

    #[test]
    fn an_unlisted_player_shows_no_listing_reason() {
        let i18n = I18n::for_test(en_map());
        let player = loan_listed_then_pathway_changed();
        assert!(ListingReasonView::of(&player, &[PlayerStatusType::Unh], &i18n).is_empty());
    }

    fn observation_date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2030, 6, day).unwrap()
    }

    fn monitoring_team(
        club_id: u32,
        slug: &str,
        kind: TeamType,
        league_id: Option<u32>,
        world_reputation: u16,
    ) -> Team {
        Team::builder()
            .id(club_id * 10 + u32::from(kind != TeamType::Main))
            .league_id(league_id)
            .club_id(club_id)
            .name(slug.to_string())
            .slug(slug.to_string())
            .team_type(kind)
            .players(PlayerCollection::new(Vec::new()))
            .staffs(StaffCollection::new(Vec::new()))
            .reputation(TeamReputation::new(500, 500, world_reputation))
            .training_schedule(TrainingSchedule::new(
                NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
            ))
            .build()
            .unwrap()
    }

    fn monitoring_club(id: u32, name: &str, teams: Vec<Team>) -> Club {
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(teams),
            ClubFacilities::default(),
        )
    }

    fn observe(club: &mut Club, scout_staff_id: u32, day: u32) {
        let date = observation_date(day);
        assert!(club.transfer_plan.keep_file(
            FileReading {
                stage: TransferMoveStage::CanScoutQuietly,
                stopped_by: None,
                read_on: date,
            },
            FileObservation {
                scout_staff_id,
                player_id: 1,
                source: ScoutMonitoringSource::StaffRecommendation,
                transfer_request_id: None,
                origin_assignment_id: None,
                region: None,
                assessed_ability: 65,
                assessed_potential: 90,
                confidence: 0.43,
                role_fit: 1.0,
                estimated_value: 0.0,
                risk_flags: Vec::new(),
                is_match: true,
            },
            date,
        ));
    }

    fn monitoring_world() -> SimulatorData {
        let mut amkar = monitoring_club(
            11,
            "Amkar",
            vec![monitoring_team(
                11,
                "amkar-perm",
                TeamType::Main,
                Some(1),
                6000,
            )],
        );
        observe(&mut amkar, 101, 14);
        observe(&mut amkar, 102, 13);

        let mut tied = monitoring_club(
            12,
            "Dinamo St Petersburg",
            vec![monitoring_team(
                12,
                "dinamo-st-petersburg",
                TeamType::Main,
                Some(1),
                6000,
            )],
        );
        observe(&mut tied, 103, 15);

        let mut reserves_first = monitoring_club(
            13,
            "Reserves First",
            vec![
                monitoring_team(13, "reserves-first-b", TeamType::B, Some(2), 9900),
                monitoring_team(13, "reserves-first", TeamType::Main, Some(3), 1000),
            ],
        );
        observe(&mut reserves_first, 104, 16);

        let mut zero = monitoring_club(
            14,
            "Zero Reputation",
            vec![monitoring_team(
                14,
                "zero-reputation",
                TeamType::Main,
                None,
                0,
            )],
        );
        observe(&mut zero, 105, 12);

        let mut no_team = monitoring_club(15, "No Senior Team", Vec::new());
        for club in [&mut amkar, &mut no_team] {
            assert!(club.transfer_plan.shortlist(
                FileReading {
                    stage: TransferMoveStage::CanShowPublicInterest,
                    stopped_by: None,
                    read_on: observation_date(18),
                },
                1,
                0.0,
                ShortlistCandidate {
                    player_id: 1,
                    score: 0.5,
                    estimated_fee: 0.0,
                    status: ShortlistCandidateStatus::Available,
                },
                None,
            ));
        }

        let country = monitoring_country(vec![amkar, tied, reserves_first, zero, no_team]);
        let mut data = SimulatorData::new(
            observation_date(18).and_hms_opt(12, 0, 0).unwrap(),
            vec![Continent::new(
                1,
                "Europe".to_string(),
                vec![country],
                Vec::new(),
            )],
            GlobalCompetitions::new(Vec::new()),
        );
        let player = loan_listed_then_pathway_changed();
        let mut unmonitored = loan_listed_then_pathway_changed();
        unmonitored.id = 2;
        data.free_agents.push(player);
        data.free_agents.push(unmonitored);
        data
    }

    fn monitoring_country(clubs: Vec<Club>) -> Country {
        let league = |id, name: &str, slug: &str| {
            League::new(
                id,
                name.to_string(),
                slug.to_string(),
                1,
                3000,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                    split_season: false,
                },
                false,
            )
        };
        Country::builder()
            .id(1)
            .code("ru".to_string())
            .slug("russia".to_string())
            .name("Russia".to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![
                league(
                    1,
                    "Second Division B Ural-Volga",
                    "russian-second-division-b-ural-volga",
                ),
                league(2, "Reserve League", "reserve-league"),
                league(3, "Senior League", ""),
            ]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    #[test]
    fn monitoring_uses_senior_league_and_stable_world_reputation_order() {
        let data = monitoring_world();
        let player = data.player(1).unwrap();
        let i18n = crate::I18nManager::new().for_lang("en");
        let source = data.player_monitoring_details(player.id);
        assert_eq!(source[0].club_name, "Reserves First");
        let rows = TransfersView::monitoring(&data, player, &i18n);

        let ranks: Vec<_> = rows
            .iter()
            .map(|row| (row.club_name.as_str(), row.scout_id, row.world_reputation))
            .collect();
        assert_eq!(
            ranks,
            [
                ("Dinamo St Petersburg", 103, Some(6000)),
                ("Amkar", 101, Some(6000)),
                ("Amkar", 102, Some(6000)),
                ("Reserves First", 104, Some(1000)),
                ("Zero Reputation", 105, Some(0)),
                ("No Senior Team", 0, None),
            ]
        );
        assert_eq!(
            rows[3].league.as_ref().unwrap().name,
            "Russian Senior League"
        );
        assert!(rows[4].league.is_none());
        assert!(rows[5].league.is_none());
    }

    #[test]
    fn transfer_routes_render_localized_club_flags_and_monitoring() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let _runtime_guard = runtime.enter();
        let i18n = Arc::new(crate::I18nManager::new());
        let mut world = monitoring_world();
        let counterparts = [
            (11, "Amkar", 1, TransferType::Permanent),
            (99, "Recorded Club", 1, TransferType::Permanent),
            (98, "Unknown Club", 0, TransferType::Permanent),
            (0, "", 1, TransferType::Free),
        ];
        world.continents[0].countries[0]
            .transfer_market
            .transfer_history
            .extend(counterparts.into_iter().map(|(id, name, origin, kind)| {
                CompletedTransfer::new(
                    500,
                    "Ledger Player".to_string(),
                    id,
                    0,
                    name.to_string(),
                    12,
                    "Dinamo St Petersburg".to_string(),
                    observation_date(16),
                    CurrencyValue::new(0.0, Currency::Usd),
                    kind,
                )
                .with_origin_country(origin)
            }));
        let state = GameAppData {
            database: Arc::new(database::DatabaseLoader::load()),
            data: Arc::new(tokio::sync::RwLock::new(Some(Arc::new(world)))),
            process_lock: Arc::default(),
            cancel_flag: Arc::default(),
            news_i18n: Arc::new(crate::NewsI18nManager::new()),
            events_i18n: Arc::new(crate::EventI18nManager::new(&i18n)),
            i18n: Arc::clone(&i18n),
            workers: crate::WorkerRegistry::empty(),
            ai: crate::AiConfig::new(),
        };
        let app = routes::PlayerTransfersRoutes::create()
            .merge(TeamTransfersRoutes::router())
            .with_state(state);
        let country_labels = [
            ("en", "Russia"),
            ("es", "Rusia"),
            ("fr", "Russie"),
            ("de", "Russland"),
            ("pt", "Rússia"),
            ("ru", "Россия"),
            ("zh", "俄羅斯"),
            ("tr", "Rusya"),
            ("ja", "ロシア"),
        ];
        for (lang, country_name) in country_labels {
            let localization = i18n.for_lang(lang);
            let label = localization.t("scouting_observations");
            assert_ne!(label, "scouting_observations");
            if lang != "en" {
                assert_ne!(label, "Scouting observations", "{lang}");
            }
            let html = runtime.block_on(transfer_page_html(
                &app,
                format!("/{lang}/players/1-seb-nava/transfers"),
            ));
            let interested = html.split_once("fm-pt-clubs\"").unwrap().1;
            let interested = interested.split_once("</div>").unwrap().0;
            let flag_label =
                format!("role=\"img\" aria-label=\"{country_name}\" title=\"{country_name}\"");
            assert_eq!(interested.matches(&flag_label).count(), 2);
            assert!(interested.contains(&format!("class=\"flag flag-ru\" {flag_label}")));
            assert!(interested.contains(&format!(
                "</span>&nbsp;<a href=\"/{lang}/teams/amkar-perm\">Amkar</a>, \
                 <span class=\"flag flag-ru fm-pt-club-flag-spaced\" {flag_label}"
            )));
            assert!(interested.contains("</span>&nbsp;No Senior Team"));
            let panel = html
                .split_once("scout-monitoring\"")
                .unwrap()
                .1
                .split_once("</section>")
                .unwrap()
                .0;
            assert!(panel.contains(&format!("aria-label=\"{label}\" tabindex=\"0\"")));
            assert!(panel.contains(&format!("href=\"/{lang}/teams/amkar-perm\"")));
            assert!(panel.contains(&format!(
                "href=\"/{lang}/leagues/russia/russian-second-division-b-ural-volga\""
            )));
            assert!(panel.contains(&format!("href=\"/{lang}/staff/101\"")));
            assert!(panel.contains("<span class=\"fm-panel-count\">6</span>"));
            assert_eq!(panel.matches("scope=\"col\"").count(), 6);
            assert_eq!(panel.matches("<td class=\"pt-club\">").count(), 6);
            assert_eq!(panel.matches(&flag_label).count(), 6, "{lang}");
            if lang == "en" {
                assert!(
                    panel
                        .contains("<span class=\"scout-club-league\">Russian Senior League</span>")
                );
                assert!(panel.contains(">Active</span>"));
                assert!(panel.contains(">Shortlisted</span>"));
                assert!(panel.contains(">43%</td>"));
                assert!(panel.contains(">1 / 1m</td>"));
                assert!(panel.contains(">14.06.2030</td>"));
                let summary = panel.split_once("<span>No Senior Team</span>").unwrap().1;
                assert!(summary.contains(&format!(
                    ">{}</td>",
                    localization.t("recruitment_department")
                )));
                assert!(summary.contains("<td class=\"pt-date\"></td>"));
                assert!(!summary.contains("href="));
            }

            let team_html = runtime.block_on(transfer_page_html(
                &app,
                format!("/{lang}/teams/dinamo-st-petersburg/transfers"),
            ));
            let club_cells: Vec<_> = team_html
                .split("<td class=\"trf-club\">")
                .skip(1)
                .map(|cell| cell.split_once("</td>").unwrap().0)
                .collect();
            assert_eq!(club_cells.len(), 4);
            for name in ["Amkar", "Recorded Club"] {
                let cell = club_cells.iter().find(|cell| cell.contains(name)).unwrap();
                assert!(cell.contains(&format!("class=\"flag flag-ru\" {flag_label}")));
            }
            let unknown = club_cells
                .iter()
                .find(|cell| cell.contains("Unknown Club"))
                .unwrap();
            let free_agent = club_cells
                .iter()
                .find(|cell| cell.contains("class=\"trf-free-agent\""))
                .unwrap();
            assert!(free_agent.contains(localization.t("free_agent")));
            assert!(!unknown.contains("role=\"img\""));
            assert!(!free_agent.contains("role=\"img\""));
        }

        let html = runtime.block_on(transfer_page_html(
            &app,
            "/en/players/2-seb-nava/transfers".into(),
        ));
        assert!(html.contains("No scouts currently watching this player"));
        assert!(!html.contains("scout-monitoring-table"));
        assert!(!html.contains("fm-pt-clubs"));
    }

    async fn transfer_page_html(app: &axum::Router, path: String) -> String {
        let response = app
            .clone()
            .oneshot(Request::get(path.as_str()).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }
}
