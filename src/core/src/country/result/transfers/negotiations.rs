use super::free_agent_market_calc::BuyerRoleFit;
use super::free_agents::{EmergencySignedTerms, GlobalFreeAgentSigning};
use super::types::{
    DeferredTransfer, NegotiationData, TransferActivitySummary, find_player_in_country,
    find_player_in_country_mut,
};
use crate::club::player::agent::PlayerAgent;
use crate::club::player::calculators::{
    ContractValuation, ValuationContext, squad_status_wage_factor,
};
use crate::club::player::events::transfer_social::{
    TransferContinentalPath, TransferInterestSignal,
};
use crate::club::team::squad::{SquadAssetClass, SquadAssetProtection};
use crate::country::result::CountryResult;
use crate::transfers::window::PlayerValuationCalculator;
use crate::transfers::NegotiationStatus;
use crate::transfers::TransferListingStatus;
use crate::transfers::TransferRoutePolicy;
use crate::transfers::TransferWindowManager;
use crate::transfers::market::TransferListingOrigin;
use crate::transfers::negotiation::{
    NegotiationPhase, NegotiationRejectionReason, TransferNegotiation,
};
use crate::transfers::offer::{PersonalTermsOffer, PromisedSquadStatus, TransferClause};
use crate::transfers::pipeline::plausibility::{
    TransferMovePlausibility, TransferPlausibilityBuilder, TransferPlausibilityEvaluator,
    TransferPlausibilityReason, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::{LoanOutReason, PipelineProcessor};
use crate::transfers::scouting_region::ScoutingRegion;
use crate::utils::{FloatUtils, FormattingUtils};
use crate::{
    Club, Country, Player, PlayerSquadStatus, PlayerStatusType, PlayerValueCalculator,
    TransferInterestSource, TransferInterestStage, WageCalculator,
};
use chrono::NaiveDate;

/// Everything one resolution tick produced beyond in-place market
/// mutations. `deferred` is the classic club-to-club execution queue;
/// the free-agent fields exist because pool players live in
/// `SimulatorData.free_agents` and their negotiations can't complete
/// (or update market state) inside the country borrow.
pub(crate) struct NegotiationOutcomes {
    pub(crate) deferred: Vec<DeferredTransfer>,
    /// Pool free agents whose negotiation cleared medical today —
    /// executed in Phase C via `execute_global_free_agent_signing`,
    /// which also writes the "Free Agent" history row (only when the
    /// player is still unclaimed, so no phantom entries).
    pub(crate) free_agent_signings: Vec<GlobalFreeAgentSigning>,
    /// Pool free agents who turned down personal terms today — bumps
    /// their `FreeAgentMarketState` rejected counter in Phase C.
    pub(crate) free_agent_rejected_ids: Vec<u32>,
}

/// Maps the negotiated `PersonalTermsOffer` back into the
/// `EmergencySignedTerms` shape `execute_global_free_agent_signing`
/// installs. Round-trips cleanly with
/// `EmergencySignedTerms::to_personal_terms`: the role only feeds the
/// squad-status promise, so a missing promise maps to `Backup` and
/// regenerates the same (absent) promise on install.
struct PoolSigningTerms;

impl PoolSigningTerms {
    fn from_personal(
        personal_terms: Option<&PersonalTermsOffer>,
        offered_annual_wage: Option<u32>,
    ) -> Option<EmergencySignedTerms> {
        let terms = personal_terms?;
        let annual_wage = terms.annual_wage.or(offered_annual_wage)?;
        let role = match terms.squad_status_promise {
            Some(PromisedSquadStatus::KeyPlayer) => BuyerRoleFit::KeyPlayer,
            Some(PromisedSquadStatus::FirstTeamRegular) => BuyerRoleFit::Starter,
            Some(PromisedSquadStatus::FirstTeamSquadRotation) => BuyerRoleFit::Rotation,
            _ => BuyerRoleFit::Backup,
        };
        Some(EmergencySignedTerms {
            annual_wage,
            contract_years: terms.contract_years.unwrap_or(1),
            role,
        })
    }
}

impl CountryResult {
    pub(crate) fn resolve_pending_negotiations(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) -> NegotiationOutcomes {
        let mut outcomes = NegotiationOutcomes {
            deferred: Vec::new(),
            free_agent_signings: Vec::new(),
            free_agent_rejected_ids: Vec::new(),
        };
        let country_id = country.id;

        // When multiple buyers have bids in flight for the same player,
        // the seller picks the strongest one first — best total
        // potential, cleanest payment structure, most credible buyer.
        // Sorting the ready-list best-first means the realistic bid
        // resolves before the also-rans, which mirrors a seller
        // explicitly comparing offers instead of accepting whatever
        // hits Medical first.
        let ready_to_resolve: Vec<u32> = SellerBidOrdering::order(country, date);

        for neg_id in ready_to_resolve {
            let neg_data = match country.transfer_market.negotiations.get(&neg_id) {
                Some(n) => {
                    let listing_ref = country.transfer_market.listings.get(n.listing_id as usize);
                    let asking_price = listing_ref.map(|l| l.asking_price.amount).unwrap_or(0.0);
                    // A market listing exists when the listing row is
                    // Available or InNegotiation. Used for plumbing —
                    // not for acceptance scoring.
                    let has_market_listing = listing_ref
                        .map(|l| {
                            l.status == TransferListingStatus::InNegotiation
                                || l.status == TransferListingStatus::Available
                        })
                        .unwrap_or(false);
                    let listing_origin = listing_ref.map(|l| l.origin);

                    // True availability is driven by the player's own
                    // status (Lst/Loa/Req/Unh/NotNeeded) or by a
                    // genuinely-seller-advertised listing. Synthetic
                    // listings created to back unsolicited approaches
                    // are explicitly excluded so that the bonuses
                    // downstream don't reward stale plumbing.
                    let player_is_available = {
                        let from_statuses =
                            super::types::find_player_in_country(country, n.player_id)
                                .map(|p| {
                                    let statuses = p.statuses.get();
                                    let listed_for_permanent =
                                        statuses.contains(&PlayerStatusType::Lst);
                                    let loaned_listed = statuses.contains(&PlayerStatusType::Loa);
                                    let requested = statuses.contains(&PlayerStatusType::Req);
                                    let unhappy = statuses.contains(&PlayerStatusType::Unh);
                                    let not_needed = p
                                        .contract
                                        .as_ref()
                                        .map(|c| {
                                            matches!(c.squad_status, PlayerSquadStatus::NotNeeded)
                                        })
                                        .unwrap_or(false);
                                    // Permanent vs loan listings count
                                    // for the corresponding move only.
                                    let listing_supports =
                                        match (n.is_loan, listed_for_permanent, loaned_listed) {
                                            (false, true, _) => true,
                                            (true, _, true) => true,
                                            _ => false,
                                        };
                                    listing_supports || requested || unhappy || not_needed
                                })
                                .unwrap_or(false);
                        let from_listing = listing_origin
                            .map(|o| {
                                matches!(
                                    o,
                                    TransferListingOrigin::SellerListed
                                        | TransferListingOrigin::LoanOutListed
                                        | TransferListingOrigin::EndOfContract
                                )
                            })
                            .unwrap_or(false);
                        from_statuses || from_listing
                    };

                    let sell_on_percentage = n.current_offer.clauses.iter().find_map(|c| {
                        if let TransferClause::SellOnClause(pct) = c {
                            Some(*pct)
                        } else {
                            None
                        }
                    });
                    NegotiationData {
                        player_id: n.player_id,
                        selling_club_id: n.selling_club_id,
                        buying_club_id: n.buying_club_id,
                        offer_amount: n.current_offer.base_fee.amount,
                        is_loan: n.is_loan,
                        has_option_to_buy: n.has_option_to_buy,
                        is_unsolicited: n.is_unsolicited,
                        phase: n.phase.clone(),
                        selling_rep: n.selling_club_reputation,
                        buying_rep: n.buying_club_reputation,
                        player_age: n.player_age,
                        player_ambition: n.player_ambition,
                        asking_price,
                        has_market_listing,
                        player_is_available,
                        listing_origin,
                        selling_country_id: n.selling_country_id,
                        selling_continent_id: n.selling_continent_id,
                        selling_country_code: n.selling_country_code.clone(),
                        player_sold_from: n.player_sold_from.clone(),
                        player_name: n.player_name.clone(),
                        selling_club_name: n.selling_club_name.clone(),
                        offered_annual_wage: n.offered_salary,
                        buying_league_reputation: n.buying_league_reputation,
                        sell_on_percentage,
                        loan_future_fee: n.current_offer.loan_future_fee().map(
                            |(fee, obligation)| (fee.amount.max(0.0).round() as u32, obligation),
                        ),
                        personal_terms: n.current_offer.personal_terms.clone(),
                        foreign_terms_floor_blocked: n.foreign_terms_floor_blocked,
                        foreign_seller_importance: n.foreign_seller_importance,
                    }
                }
                None => continue,
            };

            match neg_data.phase {
                NegotiationPhase::InitialApproach { .. } => {
                    Self::resolve_initial_approach(country, neg_id, &neg_data, date);
                }
                NegotiationPhase::ClubNegotiation { round, .. } => {
                    Self::resolve_club_negotiation(country, neg_id, &neg_data, round, date);
                }
                NegotiationPhase::PersonalTerms { .. } => {
                    Self::resolve_personal_terms(country, neg_id, &neg_data, date, &mut outcomes);
                }
                NegotiationPhase::MedicalAndFinalization { .. } => {
                    Self::resolve_medical(
                        country,
                        country_id,
                        neg_id,
                        &neg_data,
                        date,
                        summary,
                        &mut outcomes,
                    );
                }
            }
        }

        outcomes
    }

    /// Build the transfer-interest signal that the player owner needs
    /// to react to a stage transition. Composes the rep gaps, league
    /// rep gaps, rivalry, home-country, and former-club facts so the
    /// emit-side method can pick a kind / reaction without re-reading
    /// world state.
    fn build_interest_signal(
        country: &Country,
        neg_data: &NegotiationData,
        stage: TransferInterestStage,
        source: TransferInterestSource,
        repeated_attention: bool,
    ) -> Option<TransferInterestSignal> {
        let player = find_player_in_country(country, neg_data.player_id)?;
        let selling_club = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)?;
        let seller_league_rep = selling_club
            .teams
            .teams
            .first()
            .and_then(|t| t.league_id)
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let buying_club = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.buying_club_id);
        let interested_league_id =
            buying_club.and_then(|c| c.teams.teams.first().and_then(|t| t.league_id));
        let buying_country_id = country.id;
        let seller_country_id = neg_data.selling_country_id.unwrap_or(country.id);
        let is_home_country = player.country_id == buying_country_id;
        let is_seller_in_home_country = player.country_id == seller_country_id;
        let is_former_club = player
            .sold_from
            .as_ref()
            .map(|(cid, _)| *cid == neg_data.buying_club_id)
            .unwrap_or(false);
        let is_rival = selling_club.is_rival(neg_data.buying_club_id);
        let buyer_has_continental_path = BuyerContinentalPathHint {
            league_reputation: neg_data.buying_league_reputation,
        }
        .is_on_path();
        let buyer_competition_path = BuyerContinentalPathHint {
            league_reputation: neg_data.buying_league_reputation,
        }
        .competition_path(country.continent_id);
        let mut sig = TransferInterestSignal {
            interested_club_id: neg_data.buying_club_id,
            interested_league_id,
            buyer_rep: neg_data.buying_rep,
            seller_rep: neg_data.selling_rep,
            buyer_league_rep: neg_data.buying_league_reputation,
            seller_league_rep,
            stage,
            source,
            repeated_attention,
            is_rival,
            is_home_country,
            is_seller_in_home_country,
            is_former_club,
            buyer_country_id: country.id,
            buyer_continent_id: country.continent_id,
            buyer_has_continental_path,
            buyer_competition_path,
        };
        // Light helper: keep the variable mutable in case future calls
        // want to amend it before passing to the player.
        let _ = &mut sig;
        Some(sig)
    }

    /// True when the selling club has the buying club flagged as a rival.
    /// Rivalry is an acceptance-chance friction (stronger at seller's end),
    /// not a hard block — a big enough bid or a player who forces the move
    /// still gets a deal through.
    fn seller_views_buyer_as_rival(
        country: &Country,
        selling_club_id: u32,
        buying_club_id: u32,
    ) -> bool {
        country
            .clubs
            .iter()
            .find(|c| c.id == selling_club_id)
            .map(|c| c.is_rival(buying_club_id))
            .unwrap_or(false)
    }

    fn resolve_initial_approach(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) {
        // Selling club refuses to negotiate for recently signed players —
        // they bought this player with a plan and won't sell immediately.
        // Check domestic players only (foreign players aren't in this country).
        if neg_data.selling_country_id.is_none() {
            let window_mgr = TransferWindowManager::for_country(country, date);
            let current_window = window_mgr.current_window_dates(country.id, date);
            // Development-pathway bypass: the owner club itself listed
            // this same-window signing for a development loan, so loan
            // approaches are welcome. Permanent bids for the fresh
            // signing stay blocked — the protection is only relaxed for
            // the explicit pathway the owner opened.
            let development_loan_listed = neg_data.is_loan
                && country
                    .clubs
                    .iter()
                    .find(|c| c.id == neg_data.selling_club_id)
                    .map(|c| {
                        c.transfer_plan.loan_out_candidates.iter().any(|cand| {
                            cand.player_id == neg_data.player_id
                                && cand.reason == LoanOutReason::DevelopmentPathway
                        })
                    })
                    .unwrap_or(false);
            if let Some(player) = find_player_in_country(country, neg_data.player_id) {
                if !development_loan_listed && player.is_transfer_protected(date, current_window) {
                    if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id)
                    {
                        negotiation
                            .reject_with_reason(NegotiationRejectionReason::PlayerTooImportant);
                    }
                    Self::reopen_listing_for_player(country, neg_data.player_id);
                    PipelineProcessor::on_negotiation_resolved(
                        country,
                        neg_data.buying_club_id,
                        neg_data.player_id,
                        false,
                    );
                    return;
                }
            }
        }

        // Pull the shared plausibility verdict here — drives the seller
        // acceptance delta and the minimum-fee floor used below. Foreign
        // negotiations skip the verdict (we don't have the player in
        // this country's clubs to inspect).
        let plausibility = Self::plausibility_for(country, neg_data, date);
        let (seller_delta, min_fee_multiplier, importance) = match &plausibility {
            Some(TransferPlausibilityVerdict::Allow(adj)) => {
                let importance = Self::plausibility_importance(country, neg_data, date);
                (
                    adj.seller_acceptance_delta,
                    adj.minimum_fee_multiplier,
                    importance,
                )
            }
            _ => (0.0_f32, 1.0_f64, 0.0_f32),
        };

        let ratio = if neg_data.asking_price > 0.0 {
            neg_data.offer_amount / neg_data.asking_price
        } else {
            1.0
        };

        let mut chance: f32 = if neg_data.player_is_available {
            80.0
        } else if neg_data.is_unsolicited {
            35.0
        } else {
            55.0
        };

        // Reservation-price guardrails: randomness adds texture, but it
        // should not let insulting bids or unaffordable rival taps through.
        if !neg_data.is_loan && ratio < 0.45 {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::AskingPriceTooHigh);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
            return;
        }

        // Absolute seller fee floor — anchored on the player's MARKET VALUE
        // (recomputed from the selling club's context), not the listing's
        // asking price. The ratio guard above only protects against a low
        // offer relative to whatever the listing advertises; a synthetic or
        // heavily-decayed listing can advertise far below the player's worth,
        // so a 5M core player could clear the ratio test on a 340K bid. This
        // floor closes that gap. Loans, foreign moves, surplus players, and
        // typed distressed sales are exempt inside the helper.
        if let Some(floor) = SellerFeeFloor::for_permanent_domestic(country, neg_data, date) {
            if neg_data.offer_amount < floor.min_fee {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.reject_with_reason(floor.reason);
                }
                Self::reopen_listing_for_player(country, neg_data.player_id);
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    false,
                );
                return;
            }
        }

        // Important-player floor: if the player counts as important and
        // the offer doesn't clear the plausibility-driven minimum-fee
        // multiplier, the seller refuses up front. Listed or distressed
        // players slip past via the availability exemption in the
        // plausibility evaluator (min_fee_multiplier shrinks back to 1).
        if !neg_data.is_loan
            && importance >= 0.78
            && min_fee_multiplier > 1.0
            && ratio < min_fee_multiplier
        {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                let reason = if importance >= 0.90 {
                    NegotiationRejectionReason::PlayerTooImportant
                } else {
                    NegotiationRejectionReason::AskingPriceTooHigh
                };
                negotiation.reject_with_reason(reason);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
            return;
        }

        chance += seller_delta;

        if ratio >= 1.15 {
            chance += 30.0;
        } else if ratio >= 1.0 {
            chance += 22.0;
        } else if ratio >= 0.85 {
            chance += 8.0;
        } else if ratio < 0.65 {
            chance -= 22.0;
        }

        let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
        if rep_diff > 0.2 {
            chance += 15.0;
        } else if rep_diff < -0.2 {
            chance -= 10.0;
        }

        // Competition bonus: when several buyers are bidding for the
        // same player the seller has leverage and is more inclined to
        // engage with the leading offer. Read once off the market.
        // Each extra rival lifts the chance by +5, capped to avoid
        // freebie acceptances at insulting bids.
        let competing_bids = country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| {
                n.player_id == neg_data.player_id
                    && n.id != neg_id
                    && matches!(
                        n.status,
                        NegotiationStatus::Pending | NegotiationStatus::Countered
                    )
            })
            .count() as f32;
        if competing_bids > 0.0 {
            chance += (competing_bids * 5.0).min(15.0);
        }

        // Rivalry friction: seller reluctant to strengthen a rival. Softened
        // when the buyer is clearly bigger (pragmatic payday) or when the
        // bid is far above asking (can't turn down that kind of money).
        if neg_data.selling_country_id.is_none()
            && Self::seller_views_buyer_as_rival(
                country,
                neg_data.selling_club_id,
                neg_data.buying_club_id,
            )
        {
            let mut rival_penalty: f32 = 35.0;
            if rep_diff > 0.25 {
                rival_penalty -= 12.0;
            }
            if neg_data.asking_price > 0.0 && neg_data.offer_amount >= neg_data.asking_price * 1.5 {
                rival_penalty -= 15.0;
            }
            chance -= rival_penalty.max(5.0);
        }

        chance = chance.clamp(2.0, 95.0);
        let roll = FloatUtils::random(0.0, 100.0);

        if roll < chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_club_negotiation(date);
            }
            // Interest is now real — fire the structured concrete-interest
            // signal so the player owner can pick the right reaction
            // (flattered / focused / unsettled / loyal) and attach the
            // interested club, sporting fit, evidence and follow-up.
            if neg_data.selling_country_id.is_none() {
                let signal = Self::build_interest_signal(
                    country,
                    neg_data,
                    TransferInterestStage::ConcreteInterest,
                    TransferInterestSource::ConfirmedApproach,
                    false,
                );
                if let Some(sig) = signal {
                    if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                        player.on_transfer_interest_signal(&sig);
                    }
                }
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation
                    .reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            // Domestic targets feel the rejection if it was a real chance.
            // Routed through the structured interest funnel so the
            // headline carries who rejected what and the player's reaction
            // (frustration / leverage / loyalty) lands with context.
            if neg_data.selling_country_id.is_none() {
                let signal = Self::build_interest_signal(
                    country,
                    neg_data,
                    TransferInterestStage::BidRejected,
                    TransferInterestSource::RejectedBid,
                    false,
                );
                if let Some(sig) = signal {
                    if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                        player.on_transfer_interest_signal(&sig);
                    }
                }
            }
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    fn resolve_club_negotiation(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        round: u8,
        date: NaiveDate,
    ) {
        // Release clauses bypass the normal acceptance calculation entirely:
        // if a matching clause is triggered, the selling club has no choice
        // and the negotiation jumps straight to personal terms. Buy-back
        // clauses and division-tier variants are not modelled yet (they
        // need richer context than NegotiationData carries today).
        if neg_data.selling_country_id.is_none() && Self::clause_triggers_sale(country, neg_data) {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_personal_terms(date);
            }
            return;
        }

        // Absolute seller fee floor (see resolve_initial_approach). Re-checked
        // here so an offer that reaches club negotiation through any path —
        // or escalates toward a deflated synthetic asking — still can't close
        // below the player's market-value floor. A triggered release clause
        // above already short-circuited, so a forced sale is never blocked.
        if let Some(floor) = SellerFeeFloor::for_permanent_domestic(country, neg_data, date) {
            if neg_data.offer_amount < floor.min_fee {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.reject_with_reason(floor.reason);
                }
                Self::reopen_listing_for_player(country, neg_data.player_id);
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    false,
                );
                return;
            }
        }

        let ratio = if neg_data.asking_price > 0.0 {
            neg_data.offer_amount / neg_data.asking_price
        } else {
            1.0
        };
        let mut seller_reservation = if neg_data.player_is_available {
            0.82
        } else {
            1.08
        };

        // For domestic transfers, check player importance. Important players
        // require a real premium; depth players and listed players can move
        // closer to asking.
        let importance = if neg_data.selling_country_id.is_none() {
            Self::calculate_player_importance(country, neg_data.player_id, neg_data.selling_club_id)
        } else {
            // Foreign: use the seller-side importance captured at creation
            // (the seller's roster lives abroad and can't be read here), so a
            // key foreign player commands the same premium a domestic one
            // would — not a flat mid-range constant that made foreign deals
            // systematically easier and cheaper to force through.
            neg_data.foreign_seller_importance.unwrap_or(0.55)
        };
        seller_reservation += (importance as f64) * 0.28;

        if let Some(selling_club) = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)
        {
            if selling_club.finance.balance.balance < 0 {
                seller_reservation -= 0.12;
            }
        }

        let urgency = Self::deadline_urgency_for(country, date) as f64;
        if urgency > 0.0 && importance < 0.75 {
            seller_reservation -= urgency * 0.10;
        }

        let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
        if rep_diff > 0.15 && importance < 0.85 {
            seller_reservation -= 0.04;
        }

        if neg_data.selling_country_id.is_none()
            && Self::seller_views_buyer_as_rival(
                country,
                neg_data.selling_club_id,
                neg_data.buying_club_id,
            )
        {
            seller_reservation += 0.18;
        }

        seller_reservation = seller_reservation.clamp(0.55, 1.55);

        let mut chance: f32 = if ratio >= seller_reservation + 0.20 {
            92.0
        } else if ratio >= seller_reservation + 0.08 {
            78.0
        } else if ratio >= seller_reservation {
            62.0
        } else if ratio >= seller_reservation - 0.10 {
            34.0
        } else {
            8.0
        };

        chance = chance.clamp(5.0, 95.0);
        let roll = FloatUtils::random(0.0, 100.0);

        if roll < chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_personal_terms(date);
            }
        } else if round < 3 {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                // Buyer escalation rate climbs with urgency: 15% at start of
                // window, up to ~30% on the last few days (panic buy).
                // The previous offer is archived to `counter_offers` so the
                // negotiation history is auditable (who bid what, when).
                // Escalate toward the actual asking price, not
                // asking × reservation. Targeting the reservation made the
                // buyer aim exactly at the 62%-acceptance threshold, and
                // because each round only closes a fraction of the gap (with
                // a 3-round cap) the offer could never actually reach it —
                // legitimate deals timed out short of the seller's minimum.
                // Aiming at the ask gives the escalation headroom to clear
                // the reservation; SellerFeeFloor + the reservation bands
                // still gate how high the buyer realistically goes.
                let target = if neg_data.asking_price > 0.0 {
                    neg_data.asking_price
                } else {
                    negotiation.current_offer.base_fee.amount * 1.15
                };
                let escalation = 0.45 + urgency * 0.25;
                let new_amount = FormattingUtils::round_fee(
                    negotiation.current_offer.base_fee.amount
                        + (target - negotiation.current_offer.base_fee.amount).max(0.0)
                            * escalation,
                );
                let mut escalated = negotiation.current_offer.clone();
                escalated.base_fee.amount = new_amount;
                escalated.offered_date = date;
                negotiation.counter_offer(escalated);
                negotiation.advance_club_negotiation_round(date);
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::AskingPriceTooHigh);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            // Final-round rejection — the buying club really did pursue
            // and the selling club still said no. Routed through the
            // structured interest funnel so the headline can name the
            // interested club and surface the player's reaction
            // (frustrated / contract-leverage / loyal).
            if neg_data.selling_country_id.is_none() {
                let signal = Self::build_interest_signal(
                    country,
                    neg_data,
                    TransferInterestStage::BidRejected,
                    TransferInterestSource::RejectedBid,
                    true,
                );
                if let Some(sig) = signal {
                    if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                        player.on_transfer_interest_signal(&sig);
                    }
                }
            }
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    fn resolve_personal_terms(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        date: NaiveDate,
        outcomes: &mut NegotiationOutcomes,
    ) {
        let is_foreign = neg_data.selling_country_id.is_some();

        // ── Player-willingness hard floor ──
        // Before any probability roll, a player with NO availability signal
        // hard-refuses a move his career incentives reject: a clear sporting
        // step down, a lower-league move in his prime, or dropping to a
        // clearly lower-reputation club in his own market. Any genuine
        // reason to leave — a transfer request, a real listing, unhappiness,
        // a near-expiry contract, a triggered release clause — clears this
        // floor (handled inside `player_terms_floor`), leaving only the
        // probability texture below. This is the spec's "personal terms are
        // not only a probability roll" requirement: availability opens the
        // door, it does not make a first-team player accept a bad move.
        //
        // Domestic moves recompute the floor live (the seller-side data is in
        // this country). Foreign moves can't — the player's club, rank, and
        // status live abroad — so the verdict was captured from the full
        // cross-border assessment at negotiation creation and rides on the
        // negotiation. Mirrors the domestic gate so a Serie C/B side can't
        // pass personal terms with a Spartak first-teamer on a lucky roll.
        let terms_floor_blocked = if is_foreign {
            neg_data.foreign_terms_floor_blocked
        } else {
            Self::player_terms_floor_for(country, neg_data, date).is_some()
        };
        if terms_floor_blocked {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation
                    .reject_with_reason(NegotiationRejectionReason::PlayerRejectedPersonalTerms);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
            return;
        }

        // Foreign loans start lower: unfamiliar country, language, likely
        // wage cut. Players don't jump across borders for a bit-part role
        // as readily as they change clubs within their own league.
        let mut chance: f32 = if is_foreign && neg_data.is_loan {
            45.0
        } else if is_foreign {
            50.0
        } else {
            60.0
        };

        // Apply the player-side adjustment from the shared plausibility
        // layer — prime-age starters stepping down inside the same
        // domestic market get a sharp negative delta here unless the
        // availability exemption (Req/Unh/etc.) softens it.
        if let Some(TransferPlausibilityVerdict::Allow(adj)) =
            Self::plausibility_for(country, neg_data, date)
        {
            chance += adj.player_terms_delta;
        }

        // Release clause: the player almost always welcomes the move they
        // negotiated the escape route for. Overrides downward-move and
        // salary resistance below.
        if !is_foreign && Self::clause_triggers_sale(country, neg_data) {
            chance += 45.0;
        }

        // End-of-window pressure: players prefer a signed deal over an
        // expired negotiation that drops them back into limbo. Country-
        // aware variant honours non-European calendars (MLS, Argentine,
        // Russian) when computing the deadline.
        chance += Self::deadline_urgency_for(country, date) * 15.0;

        if neg_data.player_is_available {
            chance += 10.0;
        }

        let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
        if rep_diff > 0.3 {
            chance += 25.0;
        } else if rep_diff > 0.15 {
            chance += 15.0;
        } else if rep_diff < -0.3 {
            chance -= 20.0;
        } else if rep_diff < -0.15 {
            chance -= 10.0;
        }

        // Age + reputation interaction: how players at different career stages
        // evaluate upward vs downward moves
        let age = neg_data.player_age;
        let ambition = neg_data.player_ambition;

        if age < 23 {
            // Young players dream big — they want to develop at the highest level.
            // Resist downward moves strongly; welcome upward moves for development.
            if rep_diff > 0.1 {
                chance += 5.0; // Happy to move up
            } else if rep_diff < -0.3 {
                chance -= 20.0; // "I'm not throwing away my career"
            } else if rep_diff < -0.1 {
                chance -= 12.0; // Reluctant to step down
            }
        } else if age <= 28 {
            // Prime years — players want the best competition and exposure.
            // Very resistant to downward moves; moving up is welcome.
            if rep_diff > 0.15 {
                chance += 5.0;
            } else if rep_diff < -0.3 {
                chance -= 15.0; // Strong resistance in peak years
            } else if rep_diff < -0.1 {
                chance -= 10.0;
            }
        } else {
            // Veteran players — pragmatic, value playing time and money.
            // Accept downward moves more easily, especially for salary.
            chance += 5.0;
        }

        // Ambition: ambitious players dream of top clubs and resist stepping down.
        // Low-ambition players are more content wherever they are.
        if ambition > 0.7 {
            if rep_diff > 0.1 {
                chance += 10.0; // Ambitious + moving up = eager
            } else if rep_diff < -0.1 {
                // Scale penalty with the gap: bigger drop = stronger refusal
                let penalty = if rep_diff < -0.3 { 20.0 } else { 12.0 };
                chance -= penalty;
            }
        } else if ambition < 0.4 {
            // Low ambition: less bothered by prestige, more accepting
            if rep_diff < -0.1 {
                chance += 5.0;
            }
        }

        // For domestic, check salary and player-specific details
        let mut moving_to_favorite = false;
        if neg_data.selling_country_id.is_none() {
            if let Some(player) = find_player_in_country(country, neg_data.player_id) {
                // Favorite club bonus — a player moving to a childhood/legend
                // club accepts terms eagerly, and the usual downward/prestige
                // penalties are muted. Already-populated list, previously
                // ignored in every decision path.
                if player.favorite_clubs.contains(&neg_data.buying_club_id) {
                    chance += 25.0;
                    moving_to_favorite = true;
                }

                // Agent bias: greedy reps depress acceptance; loyal reps push
                // the client to stay put unless the move is a clear step up.
                let agent = PlayerAgent::for_player(player);
                chance += agent.personal_terms_delta(rep_diff);

                let current_salary = player
                    .contract
                    .as_ref()
                    .map(|c| c.salary as f64)
                    .unwrap_or(500.0);

                // Reservation wage: what the player expects to earn at the buying
                // club given their ability, age, and the destination's tier. The
                // offered wage (staged by the pipeline; falls back to a proxy of
                // the current deal if absent) is compared against this.
                //
                // Use ContractValuation so the renewal AI and transfer
                // pipeline agree on the wage curve. Best-guess destination
                // role: the same tier the player currently holds —
                // transfers within tier are the common case; promotions
                // and demotions do happen but personal terms negotiates
                // off the player's perceived market value, not a role
                // they haven't yet been promised.
                let club_rep_score = (neg_data.buying_rep).clamp(0.0, 1.0);
                let assumed_status = player
                    .contract
                    .as_ref()
                    .map(|c| c.squad_status.clone())
                    .unwrap_or(PlayerSquadStatus::FirstTeamRegular);
                let val_ctx = ValuationContext {
                    age,
                    club_reputation_score: club_rep_score,
                    league_reputation: neg_data.buying_league_reputation,
                    squad_status: assumed_status,
                    current_salary: current_salary as u32,
                    months_remaining: 24,
                    has_market_interest: true,
                };
                let reservation_wage =
                    ContractValuation::evaluate(player, &val_ctx).expected_wage as f64;
                // Underlying open-market wage referenced for guard-rails
                // below (so the silence-the-warning path stays explicit).
                let _ = WageCalculator::expected_annual_wage(
                    player,
                    age,
                    club_rep_score,
                    neg_data.buying_league_reputation,
                );
                let _ = squad_status_wage_factor; // imported for future use
                let offered_salary = neg_data
                    .offered_annual_wage
                    .map(|w| w as f64)
                    .unwrap_or_else(|| current_salary.max(500.0) * 1.05);
                let wage_gap = offered_salary / reservation_wage.max(500.0);
                let salary_ratio = offered_salary / current_salary.max(500.0);

                // Gap vs reservation wage drives the primary pass/fail signal;
                // ratio-to-current adds flavour for veterans chasing paydays.
                if wage_gap >= 1.15 {
                    chance += 15.0;
                } else if wage_gap >= 0.95 {
                    chance += 5.0;
                } else if wage_gap >= 0.80 {
                    chance -= 5.0;
                } else if wage_gap >= 0.65 {
                    chance -= 18.0;
                } else {
                    chance -= 35.0;
                }

                if salary_ratio >= 2.0 && age >= 29 {
                    chance += 10.0;
                } else if salary_ratio >= 1.3 && age >= 29 {
                    chance += 6.0;
                } else if salary_ratio < 0.8 {
                    chance -= 10.0;
                }

                let statuses = player.statuses.get();
                if statuses.contains(&PlayerStatusType::Req) {
                    chance += 25.0; // Wants out — will accept more
                } else if statuses.contains(&PlayerStatusType::Unh) {
                    chance += 20.0; // Unhappy — willing to move
                }
            }
        } else {
            // Foreign player — same age/ambition checks already applied above.
            // Add salary estimate since we can't access the actual contract.
            if rep_diff > 0.2 {
                chance += 10.0;
            }
        }

        // Player reluctance to return to a club that sold them.
        // The feeling of rejection is strong — but context matters:
        // - Sold cheaply → player felt undervalued → strong resentment
        // - Club is much bigger → prestige pull can overcome hurt pride
        // - Ambitious player → may want to prove themselves, reduces penalty
        // - Older player → more pragmatic, sentimental about returning "home"
        // - Player was unhappy/requested transfer → less resentment (they wanted out)
        if let Some((sold_club_id, sold_fee)) = &neg_data.player_sold_from {
            if *sold_club_id == neg_data.buying_club_id {
                // Base: player doesn't want to go back to club that rejected them
                let mut return_penalty: f32 = 25.0;

                // Sold cheaply relative to current offer → felt undervalued
                if *sold_fee > 0.0 && neg_data.offer_amount > sold_fee * 3.0 {
                    return_penalty += 10.0;
                }

                // Club is much bigger → prestige can overcome pride
                if rep_diff > 0.3 {
                    return_penalty -= 15.0;
                } else if rep_diff > 0.15 {
                    return_penalty -= 8.0;
                }

                // Ambitious players want to prove themselves at big clubs
                if neg_data.player_ambition > 0.7 && rep_diff > 0.1 {
                    return_penalty -= 8.0;
                }

                // Older players are more pragmatic
                if neg_data.player_age >= 30 {
                    return_penalty -= 5.0;
                }

                chance -= return_penalty.max(5.0);
            }
        }

        // Geographic preference: players resist moves to less prestigious regions.
        // A favorite-club destination overrides this — a Barca-raised kid will
        // go from Bayern to Barcelona even though W.Europe→W.Europe is a wash
        // and a prestige-drop would otherwise penalise (hypothetically).
        if let Some(sell_continent_id) = neg_data.selling_continent_id {
            let buying_region = ScoutingRegion::from_country(country.continent_id, &country.code);
            let selling_region =
                ScoutingRegion::from_country(sell_continent_id, &neg_data.selling_country_code);

            if buying_region != selling_region && !moving_to_favorite {
                let buy_prestige = buying_region.league_prestige();
                let sell_prestige = selling_region.league_prestige();
                let prestige_drop = sell_prestige - buy_prestige;

                if prestige_drop > 0.0 {
                    // Moving to less prestigious region — players resist this.
                    // Previously 60× was too soft: a 0.55 drop (W.Europe→S.America)
                    // only cost −33, leaving ~27% acceptance for prime-age players.
                    let base_penalty = prestige_drop * 110.0;

                    // Ambitious players resist prestige drops more
                    let ambition_factor = if neg_data.player_ambition > 0.7 {
                        1.5
                    } else if neg_data.player_ambition > 0.5 {
                        1.0
                    } else {
                        0.7
                    };

                    // Veterans (30+) accept drops more easily for money/playing time,
                    // but a very large drop still stings regardless of age.
                    let age_factor = if prestige_drop > 0.4 {
                        if neg_data.player_age >= 32 {
                            0.7
                        } else if neg_data.player_age >= 30 {
                            0.85
                        } else {
                            1.0
                        }
                    } else if neg_data.player_age >= 32 {
                        0.3
                    } else if neg_data.player_age >= 30 {
                        0.5
                    } else {
                        1.0
                    };

                    chance -= base_penalty * ambition_factor * age_factor;
                } else if prestige_drop < -0.1 {
                    // Moving to more prestigious region — bonus
                    chance += (-prestige_drop) * 20.0;
                }
            }
        }

        chance = chance.clamp(5.0, 95.0);
        let roll = FloatUtils::random(0.0, 100.0);

        if roll < chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_medical(date);
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation
                    .reject_with_reason(NegotiationRejectionReason::PlayerRejectedPersonalTerms);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            // A pool free agent (selling_club_id == 0) who declined the
            // terms gets it counted against his market state — this is
            // the "player actually rejected a negotiated offer" moment,
            // not the candidate scan.
            if neg_data.selling_country_id.is_none() && neg_data.selling_club_id == 0 {
                outcomes.free_agent_rejected_ids.push(neg_data.player_id);
            }
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    fn resolve_medical(
        country: &mut Country,
        country_id: u32,
        neg_id: u32,
        neg_data: &NegotiationData,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
        outcomes: &mut NegotiationOutcomes,
    ) {
        let is_foreign = neg_data.selling_country_id.is_some();
        // A staged free-agent negotiation for a player in the global
        // pool — there is no selling club (id 0) and the player isn't
        // on any roster in this country, so the at-club verification
        // and the club-to-club execution path don't apply.
        let is_pool_free_agent = !is_foreign && neg_data.selling_club_id == 0;

        // Cross-country route policy: the plausibility evaluator does not
        // run on cross-country negotiations (it operates inside one
        // country's clubs), so a Russia ↔ Ukraine bid that survived the
        // earlier scouting/shortlist filters can still arrive here. Refuse
        // it before the medical roll so a stale negotiation, restored save,
        // or alternate creation path can't complete a closed route.
        if is_foreign
            && TransferRoutePolicy::is_blocked(&neg_data.selling_country_code, &country.code, date)
        {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::CountryPairRouteBlocked);
            }
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
            return;
        }

        // Verify the player is still at the selling club (domestic) or not
        // already claimed by another deferred transfer (foreign)
        if is_foreign {
            // Reject if another negotiation for this player is already deferred
            if outcomes
                .deferred
                .iter()
                .any(|d| d.player_id == neg_data.player_id)
            {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation
                        .reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
                }
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    false,
                );
                return;
            }
        } else if is_pool_free_agent {
            // Pool membership can't be verified from country scope —
            // first-come-first-served dedup happens at execution time
            // in `execute_global_free_agent_signing`. Only guard
            // against a second pool signing staged this same tick.
            if outcomes
                .free_agent_signings
                .iter()
                .any(|s| s.player_id == neg_data.player_id)
            {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation
                        .reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
                }
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    false,
                );
                return;
            }
        } else {
            let player_at_selling_club = country
                .clubs
                .iter()
                .find(|c| c.id == neg_data.selling_club_id)
                .map(|c| c.teams.contains_player(neg_data.player_id))
                .unwrap_or(false);

            if !player_at_selling_club {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation
                        .reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
                }
                Self::reopen_listing_for_player(country, neg_data.player_id);
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    false,
                );
                return;
            }
        }

        let is_injured = if is_foreign {
            false
        } else {
            find_player_in_country(country, neg_data.player_id)
                .map(|p| p.player_attributes.is_injured)
                .unwrap_or(false)
        };
        let fail_chance = if is_injured { 8.0 } else { 1.0 };
        let roll = FloatUtils::random(0.0, 100.0);

        if roll >= fail_chance {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.accept();
            }

            // Pool free agent: completion runs in Phase C through
            // `execute_global_free_agent_signing`, which removes the
            // player from `data.free_agents` and writes the "Free
            // Agent" history row itself — writing one here as well
            // would duplicate it (or leave a phantom row when another
            // country claimed the player first).
            if is_pool_free_agent {
                let reason = country
                    .transfer_market
                    .negotiations
                    .get(&neg_id)
                    .map(|n| n.reason.clone())
                    .unwrap_or_default();
                outcomes.free_agent_signings.push(GlobalFreeAgentSigning {
                    player_id: neg_data.player_id,
                    player_name: neg_data.player_name.clone(),
                    buying_country_id: country_id,
                    buying_club_id: neg_data.buying_club_id,
                    reason,
                    terms: PoolSigningTerms::from_personal(
                        neg_data.personal_terms.as_ref(),
                        neg_data.offered_annual_wage,
                    ),
                });
                country
                    .transfer_market
                    .complete_listings_for_player(neg_data.player_id);
                country
                    .transfer_market
                    .cancel_negotiations_for_player(neg_data.player_id, neg_id);
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    true,
                );
                PipelineProcessor::clear_player_interest(country, neg_data.player_id);
                return;
            }

            // Resolve names: domestic from country, foreign from cached names
            let player_name = if is_foreign {
                neg_data.player_name.clone()
            } else {
                find_player_in_country(country, neg_data.player_id)
                    .map(|p| p.full_name.to_string())
                    .unwrap_or_default()
            };
            let from_team_name = if is_foreign {
                neg_data.selling_club_name.clone()
            } else {
                country
                    .clubs
                    .iter()
                    .find(|c| c.id == neg_data.selling_club_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default()
            };
            let to_team_name = country
                .clubs
                .iter()
                .find(|c| c.id == neg_data.buying_club_id)
                .map(|c| c.name.clone())
                .unwrap_or_default();

            if let Some(completed) = country.transfer_market.complete_transfer(
                neg_id,
                date,
                player_name,
                from_team_name,
                to_team_name,
            ) {
                summary.completed_transfers += 1;
                summary.total_fees_exchanged += completed.fee.amount;

                // All execution is deferred to SimulatorData level
                let selling_country_id = neg_data.selling_country_id.unwrap_or(country_id);
                // Reconcile the staged annual wage with the structured
                // personal-terms package: the package's wage is the
                // authoritative one if present (negotiated explicitly),
                // otherwise we fall back to the loose `offered_salary`.
                let agreed_annual_wage = neg_data
                    .personal_terms
                    .as_ref()
                    .and_then(|t| t.annual_wage)
                    .or(neg_data.offered_annual_wage);
                let offer_clauses = country
                    .transfer_market
                    .negotiations
                    .get(&neg_id)
                    .map(|n| n.current_offer.clauses.clone())
                    .unwrap_or_default();
                outcomes.deferred.push(DeferredTransfer {
                    player_id: neg_data.player_id,
                    selling_country_id,
                    selling_club_id: neg_data.selling_club_id,
                    buying_country_id: country_id,
                    buying_club_id: neg_data.buying_club_id,
                    fee: neg_data.offer_amount,
                    is_loan: neg_data.is_loan,
                    has_option_to_buy: neg_data.has_option_to_buy,
                    agreed_annual_wage,
                    buying_league_reputation: neg_data.buying_league_reputation,
                    sell_on_percentage: neg_data.sell_on_percentage,
                    loan_future_fee: neg_data.loan_future_fee,
                    personal_terms: neg_data.personal_terms.clone(),
                    offer_clauses,
                });

                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    true,
                );
                PipelineProcessor::clear_player_interest(country, neg_data.player_id);
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(NegotiationRejectionReason::MedicalFailed);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            // Late-stage collapse — both clubs and the player had agreed
            // and only the medical stood in the way. Routed through the
            // structured signal so the rendered event can name the
            // interested club and the player's reaction (excited /
            // frustrated / contract-leverage).
            if neg_data.selling_country_id.is_none() {
                let signal = Self::build_interest_signal(
                    country,
                    neg_data,
                    TransferInterestStage::MoveCollapsed,
                    TransferInterestSource::ConfirmedApproach,
                    false,
                );
                if let Some(sig) = signal {
                    if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                        player.on_transfer_interest_signal(&sig);
                    }
                }
            }
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    pub(crate) fn calculate_player_importance(
        country: &Country,
        player_id: u32,
        club_id: u32,
    ) -> f32 {
        if let Some(club) = country.clubs.iter().find(|c| c.id == club_id) {
            if club.teams.teams.is_empty() {
                return 0.5;
            }
            let team = &club.teams.teams[0];
            let players = &team.players.players;
            if players.is_empty() {
                return 0.5;
            }

            let avg_ability: f32 = players
                .iter()
                .map(|p| p.player_attributes.current_ability as f32)
                .sum::<f32>()
                / players.len() as f32;

            if let Some(player) = players.iter().find(|p| p.id == player_id) {
                let ability = player.player_attributes.current_ability as f32;
                let ratio = (ability - avg_ability) / avg_ability.max(1.0);
                let status_bonus = match &player.contract {
                    Some(c) => match c.squad_status {
                        PlayerSquadStatus::KeyPlayer => 0.3,
                        PlayerSquadStatus::FirstTeamRegular => 0.15,
                        _ => 0.0,
                    },
                    None => 0.0,
                };
                (ratio * 0.5 + 0.5 + status_bonus).clamp(0.0, 1.0)
            } else {
                0.5
            }
        } else {
            0.5
        }
    }

    pub(crate) fn reopen_listing_for_player(country: &mut Country, player_id: u32) {
        let still_has_active_bid = country.transfer_market.negotiations.values().any(|n| {
            n.player_id == player_id
                && (n.status == NegotiationStatus::Pending
                    || n.status == NegotiationStatus::Countered)
        });
        if still_has_active_bid {
            return;
        }

        for listing in &mut country.transfer_market.listings {
            if listing.player_id == player_id
                && listing.status == TransferListingStatus::InNegotiation
            {
                listing.status = TransferListingStatus::Available;
                break;
            }
        }
    }

    /// True when the offer amount meets a release clause on the player's
    /// current contract. Caller must only consult this for domestic
    /// transfers — we don't carry the foreign contract into NegotiationData.
    fn clause_triggers_sale(country: &Country, neg_data: &NegotiationData) -> bool {
        let player = match find_player_in_country(country, neg_data.player_id) {
            Some(p) => p,
            None => return false,
        };
        let contract = match player.contract.as_ref() {
            Some(c) => c,
            None => return false,
        };
        // Buyer is foreign to the selling club when the negotiation was
        // cross-country to start with — that's captured by selling_country_id
        // being Some. Domestic negotiations resolve within this country.
        let buyer_is_foreign = neg_data.selling_country_id.is_some();
        contract
            .release_clause_triggered(neg_data.offer_amount, buyer_is_foreign)
            .is_some()
    }

    /// Build the plausibility verdict for the current negotiation.
    /// Returns `None` for cross-country deals (we don't carry the
    /// selling-side context here) or when the buying / selling club /
    /// player can't be located in this country.
    fn plausibility_for(
        country: &Country,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) -> Option<TransferPlausibilityVerdict> {
        if neg_data.selling_country_id.is_some() {
            return None;
        }
        let buyer = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.buying_club_id)?;
        let seller = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)?;
        let player = find_player_in_country(country, neg_data.player_id)?;
        let inputs = TransferPlausibilityBuilder::from_clubs(
            country,
            buyer,
            seller,
            player,
            neg_data.asking_price.max(neg_data.offer_amount),
            neg_data.is_loan,
            neg_data.is_unsolicited,
            date,
        );
        Some(TransferPlausibilityEvaluator::evaluate(&inputs))
    }

    /// The player-willingness hard floor for the current (domestic)
    /// negotiation. `Some(reason)` means the player would refuse the move
    /// outright at the personal-terms phase regardless of the wage offer;
    /// `None` means the move is basically reasonable for him (or he has a
    /// real reason to leave that clears the floor). Cross-country deals
    /// return `None` — the selling-side context isn't carried here, and the
    /// foreign personal-terms path keeps its own prestige/region logic.
    fn player_terms_floor_for(
        country: &Country,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) -> Option<TransferPlausibilityReason> {
        if neg_data.selling_country_id.is_some() {
            return None;
        }
        let buyer = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.buying_club_id)?;
        let seller = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)?;
        let player = find_player_in_country(country, neg_data.player_id)?;
        let inputs = TransferPlausibilityBuilder::from_clubs(
            country,
            buyer,
            seller,
            player,
            neg_data.asking_price.max(neg_data.offer_amount),
            neg_data.is_loan,
            neg_data.is_unsolicited,
            date,
        );
        TransferMovePlausibility::player_terms_floor(&inputs)
    }

    /// Convenience: rebuild the same inputs and read off the importance
    /// score. Used by the initial-approach floor that protects key
    /// players from cheap bids.
    fn plausibility_importance(
        country: &Country,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) -> f32 {
        if neg_data.selling_country_id.is_some() {
            return 0.0;
        }
        let buyer = match country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.buying_club_id)
        {
            Some(c) => c,
            None => return 0.0,
        };
        let seller = match country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)
        {
            Some(c) => c,
            None => return 0.0,
        };
        let player = match find_player_in_country(country, neg_data.player_id) {
            Some(p) => p,
            None => return 0.0,
        };
        let inputs = TransferPlausibilityBuilder::from_clubs(
            country,
            buyer,
            seller,
            player,
            neg_data.asking_price.max(neg_data.offer_amount),
            neg_data.is_loan,
            neg_data.is_unsolicited,
            date,
        );
        TransferPlausibilityEvaluator::player_importance(&inputs)
    }

    /// Country-aware variant of [`Self::deadline_urgency`]. Reads the
    /// country code to pick the right calendar so e.g. an MLS-style or
    /// southern-hemisphere window's deadline registers correctly.
    /// Callers in the country-result transfer flow have a `&Country` in
    /// scope and prefer this.
    pub(crate) fn deadline_urgency_for(country: &Country, date: NaiveDate) -> f32 {
        let mgr = TransferWindowManager::for_country(country, date);
        Self::deadline_urgency_from_manager(&mgr, country.id, date)
    }

    /// How close are we to the transfer window slamming shut? 0 when at
    /// least two weeks remain; ramps linearly to 1.0 on deadline day.
    /// Used to push both sides toward a deal instead of letting stale
    /// negotiations age into the expiry rejection. Falls back to default
    /// European windows when only the id is known. The country-aware
    /// variant `deadline_urgency_for` is preferred when a `&Country`
    /// reference is in scope.
    #[allow(dead_code)]
    pub(crate) fn deadline_urgency(country_id: u32, date: NaiveDate) -> f32 {
        let mgr = TransferWindowManager::new();
        Self::deadline_urgency_from_manager(&mgr, country_id, date)
    }

    fn deadline_urgency_from_manager(
        mgr: &TransferWindowManager,
        country_id: u32,
        date: NaiveDate,
    ) -> f32 {
        let (_, end) = match mgr.current_window_dates(country_id, date) {
            Some(w) => w,
            None => return 0.0,
        };
        let days_left = (end - date).num_days();
        if days_left >= 14 {
            0.0
        } else if days_left <= 1 {
            1.0
        } else {
            1.0 - (days_left as f32 - 1.0) / 13.0
        }
    }
}

/// How strong a *typed* reason the seller has to part with a player below his
/// market value. Drives how far the [`SellerFeeFloor`] is lowered. Every
/// level is reached only from a concrete piece of state — never a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SellerDistress {
    /// No typed reason to undersell — the full importance floor applies.
    None,
    /// A market-leverage signal (a genuine transfer listing, or unhappiness):
    /// the buyer knows the player is gettable, so the floor drops a little —
    /// but a valuable first-teamer still can't leave for a fraction of his
    /// worth. This is the "listed status reduces price only modestly" rule.
    Modest,
    /// A strong, typed reason the player should/can move below value: a formal
    /// transfer request, a `NotNeeded` surplus tag, a contract running down, a
    /// club in financial crisis, or a durable unresolved grievance. Only the
    /// low distressed residual floor remains.
    Strong,
}

/// Seller-side **absolute** fee floor for a permanent, same-country sale.
///
/// The negotiation resolvers judge an incoming bid on the offer ÷ asking
/// RATIO. That ratio is only as honest as the listing it reads: a synthetic
/// listing fabricated to back an unsolicited approach — or one heavily
/// decayed by the stale-listing logic — can advertise an asking price far
/// below the player's worth, letting an important player be prised away for a
/// fraction of his value (the Litvinov → Baltika 340K bug). This floor closes
/// that gap. It is anchored on the player's MARKET VALUE, recomputed from the
/// SELLING club's league/club context via [`PlayerValueCalculator`] (never
/// the listing), so a low asking price can no longer define a cheap sale.
///
/// The floor only bites for players the club would not cheaply part with
/// (core / first-team-useful / rotation), classified by the central
/// [`SquadAssetProtection`] policy. Genuine surplus and development players
/// carry no importance floor — their cheaper sales are governed by the ratio
/// and plausibility layers. A typed [`SellerDistress`] reason lowers the floor
/// (and, for an unrated player, removes it), so real fire-sales still clear —
/// but a valuable asset always keeps a low residual floor rather than being
/// handed over for pennies.
pub(crate) struct SellerFeeFloor;

/// The floor decision for one negotiation: the minimum acceptable base fee
/// plus the supporting context (exposed so the rule is unit-testable and the
/// rejection can be narrated). The context fields are read by the floor's
/// tests and kept for decision diagnostics.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct SellerFloorVerdict {
    /// Lowest permanent base fee the seller will accept.
    pub(crate) min_fee: f64,
    /// The player's recomputed market value (seller context, no listing
    /// discount) — the anchor the floor is a fraction of.
    pub(crate) market_value: f64,
    /// Fraction of `market_value` the floor sits at.
    pub(crate) fraction: f64,
    pub(crate) asset_class: SquadAssetClass,
    pub(crate) distress: SellerDistress,
    /// Reason to stamp on the rejection when a bid falls short.
    pub(crate) reason: NegotiationRejectionReason,
}

impl SellerFeeFloor {
    /// Healthy-club floors as a fraction of market value, by how protected the
    /// player is. A de-facto starter / `KeyPlayer` demands the most; rotation
    /// depth the least.
    const CORE_FLOOR: f64 = 0.80;
    const FIRST_TEAM_FLOOR: f64 = 0.70;
    const ROTATION_FLOOR: f64 = 0.40;
    /// Discount applied by a modest market-leverage signal (genuine listing /
    /// unhappiness). The buyer's knowledge that the player is available is
    /// worth a little, not a giveaway.
    const MODEST_DISCOUNT: f64 = 0.10;
    /// The low residual floor a valued player keeps even under a strong,
    /// typed distressed reason: a fire-sale is allowed, an insult is not.
    const DISTRESSED_RESIDUAL: f64 = 0.30;
    /// Durable-unhappiness thresholds — mirror the listing pass's
    /// `unhappiness_is_durable` so the two systems agree on what "settled,
    /// structural unhappiness" means (vs. a passing playing-time dip).
    const DURABLE_UNHAPPY_STREAK: u8 = 6;
    const DURABLE_AMBITION_FIT: f32 = -8.0;

    /// The floor for the current negotiation, or `None` when none applies (a
    /// loan, a cross-country move where the seller-side data lives abroad, an
    /// unrated surplus/development player, or an unrated player with no
    /// distressed reason). The caller rejects any permanent bid below
    /// `min_fee`.
    pub(crate) fn for_permanent_domestic(
        country: &Country,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) -> Option<SellerFloorVerdict> {
        if neg_data.is_loan || neg_data.selling_country_id.is_some() {
            return None;
        }
        let seller = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)?;
        let player = find_player_in_country(country, neg_data.player_id)?;

        let asset_class = SquadAssetProtection::classify(player, seller, date);
        let distress = Self::distress_for(player, seller, date);
        let fraction = Self::floor_fraction(asset_class, distress)?;

        let (league_rep, club_rep) = PlayerValuationCalculator::seller_context(country, seller);
        let market_value = PlayerValueCalculator::calculate(
            player,
            date,
            country.settings.pricing.price_level,
            league_rep,
            club_rep,
        );

        let reason = match asset_class {
            SquadAssetClass::CorePlayer => NegotiationRejectionReason::PlayerTooImportant,
            _ => NegotiationRejectionReason::AskingPriceTooHigh,
        };

        Some(SellerFloorVerdict {
            min_fee: market_value * fraction,
            market_value,
            fraction,
            asset_class,
            distress,
            reason,
        })
    }

    /// Floor fraction for a (class, distress) pair. `None` = no floor at all
    /// (an unrated surplus/development/unknown player with no distressed
    /// reason: the ratio + plausibility layers govern his cheaper sale).
    fn floor_fraction(asset_class: SquadAssetClass, distress: SellerDistress) -> Option<f64> {
        // Importance premium the player commands by virtue of his standing.
        let importance = match asset_class {
            SquadAssetClass::CorePlayer => Self::CORE_FLOOR,
            SquadAssetClass::FirstTeamUseful => Self::FIRST_TEAM_FLOOR,
            SquadAssetClass::RotationUseful => Self::ROTATION_FLOOR,
            SquadAssetClass::ProspectDevelopment
            | SquadAssetClass::TrueSurplus
            | SquadAssetClass::UnknownNeedsEvaluation => 0.0,
        };

        let after_distress = match distress {
            SellerDistress::None => importance,
            SellerDistress::Modest => (importance - Self::MODEST_DISCOUNT).max(0.0),
            // A strong, typed reason waives the importance premium entirely.
            SellerDistress::Strong => 0.0,
        };

        // A player the club rates at all, or any player a strong distressed
        // reason applies to, keeps a low residual floor so a valuable asset is
        // never handed over for pennies (a club would release him for free via
        // a different path before that). A genuinely unrated player with no
        // distress has no floor.
        let rated = importance > 0.0;
        let floor = if rated || matches!(distress, SellerDistress::Strong) {
            after_distress.max(Self::DISTRESSED_RESIDUAL)
        } else {
            after_distress
        };

        if floor > 0.0 { Some(floor) } else { None }
    }

    /// Strongest typed distressed reason in play, read entirely from concrete
    /// state (status flags, contract length, club finances, durable mood) —
    /// never decision-history strings.
    fn distress_for(player: &Player, seller: &Club, date: NaiveDate) -> SellerDistress {
        let statuses = player.statuses.get();

        // ── Strong: the player should/can leave below value. ──
        if statuses.contains(&PlayerStatusType::Req) {
            return SellerDistress::Strong;
        }
        if let Some(contract) = player.contract.as_ref() {
            if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                return SellerDistress::Strong;
            }
            let months_remaining = (contract.expiration - date).num_days() / 30;
            if months_remaining <= 6 {
                return SellerDistress::Strong;
            }
        }
        if seller.finance.balance.balance < 0 {
            return SellerDistress::Strong;
        }
        if Self::unhappiness_is_durable(player) {
            return SellerDistress::Strong;
        }

        // ── Modest: market-leverage signals (available, but not forced). ──
        if statuses.contains(&PlayerStatusType::Lst) || statuses.contains(&PlayerStatusType::Unh) {
            return SellerDistress::Modest;
        }

        SellerDistress::None
    }

    /// Long-term, structural unhappiness — a months-long unresolved mood or a
    /// settled ambition mismatch, not a passing playing-time dip. Same typed
    /// signals the listing pass reads.
    fn unhappiness_is_durable(player: &Player) -> bool {
        let happiness = &player.happiness;
        happiness.unhappy_streak >= Self::DURABLE_UNHAPPY_STREAK
            || happiness.factors.ambition_fit <= Self::DURABLE_AMBITION_FIT
    }
}

/// Builds the order in which the seller resolves multiple competing
/// bids. When two clubs both bid for the same player and both reach a
/// phase-ready point on the same tick, the seller doesn't accept
/// whichever bid hits Medical first — they compare and pick. Ordering
/// the ready-list best-first inside `resolve_pending_negotiations` gives
/// the leading bid the first opportunity to advance; the runner-up only
/// gets a chance if the leader fails its own roll. The losing bid is
/// then auto-cancelled by `complete_transfer` when the leader closes.
pub struct SellerBidOrdering;

impl SellerBidOrdering {
    /// Return ready-to-resolve negotiation ids sorted so that the
    /// seller's preferred bid resolves first within each (player_id)
    /// group. Across players the original order is preserved.
    pub fn order(country: &Country, date: NaiveDate) -> Vec<u32> {
        let mut entries: Vec<(u32, u32, f64)> = country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| n.is_phase_ready(date))
            .map(|n| (n.id, n.player_id, SellerBidValuation::score(n)))
            .collect();
        // Stable-sort by (player_id ASC, score DESC) — within a group,
        // best bid first; outside a group, order preserved.
        entries.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal))
        });
        entries.into_iter().map(|(id, _, _)| id).collect()
    }
}

/// Score a single competing bid from the seller's perspective. Higher
/// = more attractive. The formula blends:
///   * **Guaranteed cash** — base fee, weighted at 60%.
///   * **Total potential** — base + clause expected value, at 30%.
///   * **Buyer credibility** — reputation premium (a richer buyer is
///     less likely to renege on installments), at 10%.
/// Loans get a small flat score because they only ever generate the
/// loan fee — they shouldn't outrank a clean permanent bid for the
/// same player.
pub struct SellerBidValuation;

impl SellerBidValuation {
    pub fn score(negotiation: &TransferNegotiation) -> f64 {
        let offer = &negotiation.current_offer;
        let base = offer.base_fee.amount.max(0.0);
        let total = offer.total_potential_value().max(0.0);
        let buyer_rep_factor =
            1.0 + (negotiation.buying_club_reputation as f64).clamp(0.0, 1.0) * 0.15;

        // Up-front discount: heavy installment plans get a small
        // haircut because the seller carries time-value risk.
        let has_installments = offer
            .clauses
            .iter()
            .any(|c| matches!(c, TransferClause::Installments(_, _)));
        let upfront_factor = if has_installments { 0.92 } else { 1.0 };

        let raw = 0.60 * base + 0.30 * total;
        let scored = raw * upfront_factor * buyer_rep_factor;
        if negotiation.is_loan {
            // Loan offers can't outrank a permanent for the same
            // player — scale down so a healthy loan still ranks
            // above a derisory permanent bid but a credible
            // permanent bid wins comfortably.
            scored * 0.35
        } else {
            scored
        }
    }
}

/// Coarse "is this buying club on a credible continental qualification
/// path?" hint passed into `TransferInterestSignal`. Built from the
/// buying club's league reputation only — the negotiation pipeline
/// doesn't carry per-club continental cup state, so the hint stays
/// reputation-driven on purpose. Threshold tuning is intentionally
/// narrower than the desire-context heuristic on the player side
/// because we have less information here (no league position).
pub struct BuyerContinentalPathHint {
    pub league_reputation: u16,
}

impl BuyerContinentalPathHint {
    /// True when the buyer's league sits at a level where mid-table
    /// finishers still see continental cup minutes.
    pub fn is_on_path(&self) -> bool {
        self.league_reputation >= 6500
    }

    /// Map league reputation × continent to a coarse continental tier
    /// the buyer can offer the player. Returns `None` for non-European
    /// non-South-American leagues — those don't carry the "ambition
    /// satisfaction" semantics on the new opportunity kinds.
    pub fn competition_path(&self, continent_id: u32) -> Option<TransferContinentalPath> {
        const EUROPE: u32 = 1;
        const SOUTH_AMERICA: u32 = 3;
        match continent_id {
            EUROPE => Some(match self.league_reputation {
                r if r >= 8500 => TransferContinentalPath::EliteEurope,
                r if r >= 7000 => TransferContinentalPath::EuropaLeague,
                r if r >= 5500 => TransferContinentalPath::ConferenceLeague,
                _ => return None,
            }),
            SOUTH_AMERICA => Some(match self.league_reputation {
                r if r >= 6500 => TransferContinentalPath::Libertadores,
                r if r >= 4500 => TransferContinentalPath::Sudamericana,
                _ => return None,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod deadline_urgency_tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn no_urgency_when_window_is_closed() {
        // Default summer window is Jun 1 – Aug 31; March is outside.
        assert_eq!(CountryResult::deadline_urgency(1, d(2025, 3, 15)), 0.0);
    }

    #[test]
    fn no_urgency_early_in_window() {
        // Jun 5 is early summer window, plenty of time left.
        assert_eq!(CountryResult::deadline_urgency(1, d(2025, 6, 5)), 0.0);
    }

    #[test]
    fn urgency_ramps_up_in_final_two_weeks() {
        // Aug 31 = deadline; Aug 25 ≈ 6 days left.
        let six_days = CountryResult::deadline_urgency(1, d(2025, 8, 25));
        assert!(six_days > 0.4 && six_days < 0.8, "got {six_days}");
    }

    #[test]
    fn urgency_peaks_on_deadline_day() {
        let last = CountryResult::deadline_urgency(1, d(2025, 8, 31));
        assert!(last >= 0.95, "got {last}");
    }

    #[test]
    fn urgency_monotonic_across_window() {
        let a = CountryResult::deadline_urgency(1, d(2025, 8, 18));
        let b = CountryResult::deadline_urgency(1, d(2025, 8, 25));
        let c = CountryResult::deadline_urgency(1, d(2025, 8, 30));
        assert!(a < b);
        assert!(b < c);
    }
}

#[cfg(test)]
mod seller_bid_valuation_tests {
    use super::*;
    use crate::shared::{Currency, CurrencyValue};
    use crate::transfers::negotiation::TransferNegotiation;
    use crate::transfers::offer::{TransferClause, TransferOffer};
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn money(amount: f64) -> CurrencyValue {
        CurrencyValue {
            amount,
            currency: Currency::Usd,
        }
    }

    fn negotiation(amount: f64, is_loan: bool, has_installments: bool) -> TransferNegotiation {
        let mut offer = TransferOffer::new(money(amount), 99, d(2026, 7, 1));
        if has_installments {
            offer = offer.with_clause(TransferClause::Installments(money(amount * 0.5), 3));
        }
        let mut n =
            TransferNegotiation::new(1, 10, 0, 1, 2, offer, d(2026, 7, 1), 0.5, 0.5, 24, 0.5);
        n.is_loan = is_loan;
        n
    }

    #[test]
    fn higher_base_fee_scores_higher() {
        let lo = negotiation(2_000_000.0, false, false);
        let hi = negotiation(5_000_000.0, false, false);
        assert!(SellerBidValuation::score(&hi) > SellerBidValuation::score(&lo));
    }

    #[test]
    fn installment_heavy_bid_loses_to_clean_upfront_at_same_headline() {
        // Same headline fee but one is paid in installments → seller
        // prefers the upfront one even though headline is identical.
        let upfront = negotiation(5_000_000.0, false, false);
        let in_install = negotiation(5_000_000.0, false, true);
        assert!(
            SellerBidValuation::score(&upfront) > SellerBidValuation::score(&in_install),
            "upfront should outrank installment-heavy at equal fee"
        );
    }

    #[test]
    fn loan_bid_never_outranks_credible_permanent_for_same_player() {
        let loan = negotiation(5_000_000.0, true, false);
        let permanent = negotiation(4_000_000.0, false, false);
        // Smaller permanent fee still beats larger loan because loans
        // get the loan-flag haircut — sellers prefer the actual sale.
        assert!(
            SellerBidValuation::score(&permanent) > SellerBidValuation::score(&loan),
            "permanent bid should outrank a loan bid for the same player"
        );
    }
}

#[cfg(test)]
mod development_pathway_protection_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::fullname::FullName;
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::offer::TransferOffer;
    use crate::transfers::pipeline::{LoanOutCandidate, LoanOutStatus};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerCollection, PlayerPlan, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    /// Fixtures for the same-window / signing-plan protection bypass.
    /// Wrapped in a unit struct per the project's no-free-helpers rule.
    struct ProtectionFixtures;

    impl ProtectionFixtures {
        const SELLER_ID: u32 = 1;
        const BUYER_ID: u32 = 2;
        const PLAYER_ID: u32 = 100;
        const NEG_ID: u32 = 50;

        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn date() -> NaiveDate {
            Self::d(2026, 7, 10)
        }

        /// Fresh signing under an active Development plan — protected
        /// from outbound moves by `is_transfer_protected`.
        fn protected_prospect() -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 60;
            let mut player = PlayerBuilder::new()
                .id(Self::PLAYER_ID)
                .full_name(FullName::new("Dev".to_string(), "Prospect".to_string()))
                .birth_date(Self::d(2008, 1, 1))
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::Striker,
                        level: 16,
                    }],
                })
                .player_attributes(attrs)
                .build()
                .unwrap();
            player.plan = Some(PlayerPlan::from_signing(
                18,
                2_000_000.0,
                Self::d(2026, 7, 1),
            ));
            player
        }

        fn club(id: u32, name: &str, players: Vec<Player>) -> Club {
            let team = Team::builder()
                .id(id * 10)
                .league_id(Some(10))
                .club_id(id)
                .name(name.to_string())
                .slug(format!("club-{id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(5000, 5000, 5000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            Club::new(
                id,
                name.to_string(),
                Location::new(1),
                ClubFinances::new(1_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }

        /// Country with the protected prospect at the seller plus an open
        /// loan negotiation from the buyer. `dev_listed` controls whether
        /// the seller has staged the DevelopmentPathway candidate.
        fn world(dev_listed: bool) -> Country {
            let seller = {
                let mut club =
                    Self::club(Self::SELLER_ID, "Seller", vec![Self::protected_prospect()]);
                if dev_listed {
                    club.transfer_plan
                        .loan_out_candidates
                        .push(LoanOutCandidate {
                            player_id: Self::PLAYER_ID,
                            reason: LoanOutReason::DevelopmentPathway,
                            status: LoanOutStatus::Listed,
                            loan_fee: 0.0,
                        });
                }
                club
            };
            let buyer = Self::club(Self::BUYER_ID, "Borrower", Vec::new());

            let league = League::new(
                10,
                "L".to_string(),
                "league".to_string(),
                1,
                5000,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            );
            let mut country = Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("england".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![seller, buyer])
                .build()
                .unwrap();

            let offer = TransferOffer::new(
                CurrencyValue::new(50_000.0, Currency::Usd),
                Self::BUYER_ID,
                Self::date(),
            );
            let mut neg = TransferNegotiation::new(
                Self::NEG_ID,
                Self::PLAYER_ID,
                0,
                Self::SELLER_ID,
                Self::BUYER_ID,
                offer,
                Self::date(),
                500.0,
                500.0,
                18,
                10.0,
            );
            neg.is_loan = true;
            country
                .transfer_market
                .negotiations
                .insert(Self::NEG_ID, neg);

            country
        }

        fn neg_data(country: &Country) -> NegotiationData {
            let phase = country.transfer_market.negotiations[&Self::NEG_ID]
                .phase
                .clone();
            NegotiationData {
                player_id: Self::PLAYER_ID,
                selling_club_id: Self::SELLER_ID,
                buying_club_id: Self::BUYER_ID,
                offer_amount: 50_000.0,
                is_loan: true,
                has_option_to_buy: false,
                is_unsolicited: false,
                phase,
                selling_rep: 500.0,
                buying_rep: 500.0,
                player_age: 18,
                player_ambition: 10.0,
                asking_price: 60_000.0,
                has_market_listing: true,
                player_is_available: true,
                listing_origin: None,
                selling_country_id: None,
                selling_continent_id: None,
                selling_country_code: String::new(),
                player_sold_from: None,
                player_name: "Dev Prospect".to_string(),
                selling_club_name: "Seller".to_string(),
                offered_annual_wage: Some(50_000),
                buying_league_reputation: 5000,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms: None,
                foreign_terms_floor_blocked: false,
                foreign_seller_importance: None,
            }
        }
    }

    /// Without the owner-staged development candidate, the protected
    /// fresh signing refuses every approach — including loans.
    #[test]
    fn protected_signing_without_dev_listing_rejects_loan_approach() {
        let mut country = ProtectionFixtures::world(false);
        let neg_data = ProtectionFixtures::neg_data(&country);

        CountryResult::resolve_initial_approach(
            &mut country,
            ProtectionFixtures::NEG_ID,
            &neg_data,
            ProtectionFixtures::date(),
        );

        let neg = &country.transfer_market.negotiations[&ProtectionFixtures::NEG_ID];
        assert_eq!(
            neg.rejection_reason,
            Some(NegotiationRejectionReason::PlayerTooImportant),
            "same-window/plan protection must reject loans for non-listed signings"
        );
    }

    /// With the DevelopmentPathway candidate staged by the owner, the
    /// loan approach gets past the protection gate. The negotiation may
    /// still resolve either way downstream (acceptance chance rolls),
    /// but it must never die on PlayerTooImportant.
    #[test]
    fn dev_listed_signing_lets_loan_approach_past_protection() {
        let mut country = ProtectionFixtures::world(true);
        let neg_data = ProtectionFixtures::neg_data(&country);

        CountryResult::resolve_initial_approach(
            &mut country,
            ProtectionFixtures::NEG_ID,
            &neg_data,
            ProtectionFixtures::date(),
        );

        let neg = &country.transfer_market.negotiations[&ProtectionFixtures::NEG_ID];
        assert_ne!(
            neg.rejection_reason,
            Some(NegotiationRejectionReason::PlayerTooImportant),
            "owner-listed development loan must bypass the protection gate"
        );
    }
}

/// Regression coverage for the underpriced-important-player sale (the
/// Litvinov → Baltika 340K bug). Exercises the [`SellerFeeFloor`] directly
/// for the typed scenarios and drives one full `resolve_initial_approach`
/// to confirm the wiring rejects a sub-floor bid.
#[cfg(test)]
mod seller_fee_floor_tests {
    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings, Season};
    use crate::shared::fullname::FullName;
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::negotiation::TransferNegotiation;
    use crate::transfers::offer::TransferOffer;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, PersonAttributes, Player,
        PlayerAttributes, PlayerClubContract, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, PlayerStatistics, PlayerStatisticsHistoryItem,
        StaffCollection, Team, TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::{Datelike, NaiveTime};

    /// Fixtures for the fee-floor tests. Spartak-like seller (id 1, strong
    /// league/club reputation) and Baltika-like buyer (id 2, smaller).
    struct Ff;

    impl Ff {
        const SELLER_ID: u32 = 1;
        const BUYER_ID: u32 = 2;
        const NEG_ID: u32 = 77;
        const LEAGUE_ID: u32 = 1;

        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }
        fn date() -> NaiveDate {
            Self::d(2026, 7, 10)
        }
        fn far_contract() -> NaiveDate {
            Self::d(2030, 6, 30)
        }

        /// A central midfielder with explicit CA / age / reputation / squad
        /// status / contract expiry. Skills are the builder default — the
        /// valuation only needs them present, not tuned.
        fn player(
            id: u32,
            ca: u8,
            age: u8,
            rep: i16,
            status: PlayerSquadStatus,
            expiration: NaiveDate,
        ) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = ca;
            attrs.potential_ability = ca;
            attrs.current_reputation = rep;
            attrs.home_reputation = rep;
            attrs.world_reputation = rep;
            let mut contract = PlayerClubContract::new(50_000, expiration);
            contract.squad_status = status;
            let birth_year = Self::date().year() - age as i32;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Test".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(birth_year, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::default())
                .positions(PlayerPositions {
                    positions: vec![PlayerPosition {
                        position: PlayerPositionType::MidfielderCenter,
                        level: 18,
                    }],
                })
                .player_attributes(attrs)
                .contract(Some(contract))
                .build()
                .unwrap()
        }

        fn team(club_id: u32, players: Vec<Player>) -> Team {
            Team::builder()
                .id(club_id * 10)
                .league_id(Some(Self::LEAGUE_ID))
                .club_id(club_id)
                .name(format!("club-{club_id}"))
                .slug(format!("club-{club_id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(6000, 6000, 6000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(id: u32, name: &str, balance: i64, players: Vec<Player>) -> Club {
            Club::new(
                id,
                name.to_string(),
                Location::new(1),
                ClubFinances::new(balance, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![Self::team(id, players)]),
                ClubFacilities::default(),
            )
        }

        /// A country with the seller squad and an (empty) buyer club.
        fn country(seller_players: Vec<Player>) -> Country {
            Self::country_with_balance(seller_players, 10_000_000)
        }

        fn country_with_balance(seller_players: Vec<Player>, seller_balance: i64) -> Country {
            let seller = Self::club(Self::SELLER_ID, "Spartak", seller_balance, seller_players);
            let buyer = Self::club(Self::BUYER_ID, "Baltika", 1_000_000, Vec::new());
            let league = League::new(
                Self::LEAGUE_ID,
                "RPL".to_string(),
                "rpl".to_string(),
                1,
                6000,
                LeagueSettings {
                    season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                    season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                    tier: 1,
                    promotion_spots: 0,
                    relegation_spots: 0,
                    league_group: None,
                },
                false,
            );
            Country::builder()
                .id(1)
                .code("ru".to_string())
                .slug("russia".to_string())
                .name("Russia".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![seller, buyer])
                .build()
                .unwrap()
        }

        /// Throwaway negotiation phase (InitialApproach) — the floor helper
        /// never reads it, but `NegotiationData` requires one.
        fn initial_phase() -> NegotiationPhase {
            let offer = TransferOffer::new(CurrencyValue::new(1.0, Currency::Usd), 2, Self::date());
            TransferNegotiation::new(1, 1, 0, 1, 2, offer, Self::date(), 0.5, 0.5, 26, 0.5)
                .phase
                .clone()
        }

        /// A permanent, domestic, unsolicited `NegotiationData` for the
        /// target with the given offer and (possibly laundered) asking price.
        fn neg_data(player_id: u32, offer: f64, asking: f64) -> NegotiationData {
            NegotiationData {
                player_id,
                selling_club_id: Self::SELLER_ID,
                buying_club_id: Self::BUYER_ID,
                offer_amount: offer,
                is_loan: false,
                has_option_to_buy: false,
                is_unsolicited: true,
                phase: Self::initial_phase(),
                selling_rep: 0.75,
                buying_rep: 0.40,
                player_age: 26,
                player_ambition: 0.5,
                asking_price: asking,
                has_market_listing: true,
                player_is_available: false,
                listing_origin: None,
                selling_country_id: None,
                selling_continent_id: None,
                selling_country_code: String::new(),
                player_sold_from: None,
                player_name: "Target".to_string(),
                selling_club_name: "Spartak".to_string(),
                offered_annual_wage: Some(50_000),
                buying_league_reputation: 6000,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms: None,
                foreign_terms_floor_blocked: false,
                foreign_seller_importance: None,
            }
        }

        fn history_row(year: u16, games: u16) -> PlayerStatisticsHistoryItem {
            let mut stats = PlayerStatistics::default();
            stats.played = games;
            PlayerStatisticsHistoryItem {
                season: Season::new(year),
                team_name: "T".to_string(),
                team_slug: "t".to_string(),
                team_reputation: 0,
                league_name: "L".to_string(),
                league_slug: "l".to_string(),
                is_loan: false,
                transfer_fee: None,
                statistics: stats,
                seq_id: year as u32,
            }
        }
    }

    /// Regression #1: a Spartak-like seller's core midfielder (value in the
    /// millions) carries a strong fee floor; a Baltika-like 340K bid is far
    /// below it. The floor is anchored on the SELLER's market value, not the
    /// listing's asking price.
    #[test]
    fn core_player_floor_blocks_lowball_bid() {
        let target =
            Ff::player(100, 150, 26, 5000, PlayerSquadStatus::KeyPlayer, Ff::far_contract());
        let country = Ff::country(vec![target]);
        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);

        let v = SellerFeeFloor::for_permanent_domestic(&country, &nd, Ff::date())
            .expect("a core player must carry a fee floor");
        assert_eq!(v.asset_class, SquadAssetClass::CorePlayer);
        assert_eq!(v.distress, SellerDistress::None);
        assert_eq!(v.fraction, SellerFeeFloor::CORE_FLOOR);
        assert!(
            v.market_value > 1_000_000.0,
            "a CA-150 player must be valued in the millions, got {}",
            v.market_value
        );
        assert_eq!(v.min_fee, v.market_value * SellerFeeFloor::CORE_FLOOR);
        assert!(
            340_000.0 < v.min_fee,
            "a 340K bid ({}) must sit far below the core floor ({})",
            340_000.0,
            v.min_fee
        );
        assert_eq!(v.reason, NegotiationRejectionReason::PlayerTooImportant);
    }

    /// Regression #2: end-to-end — a low-budget unsolicited 340K bid for a
    /// valued first-team player is rejected by `resolve_initial_approach`,
    /// deterministically (before any acceptance roll). The target is a
    /// recent regular tagged `NotYetSet`, so the *plausibility* importance
    /// is below 0.78 and the pre-existing importance gate does NOT fire —
    /// only the new market-value floor catches the bid.
    #[test]
    fn low_budget_bid_for_protected_player_is_rejected_end_to_end() {
        let mut target =
            Ff::player(100, 130, 26, 2000, PlayerSquadStatus::NotYetSet, Ff::far_contract());
        target.statistics_history.items.push(Ff::history_row(2025, 14)); // recent regular
        target.statistics.played = 2; // thin current sample
        let better_a =
            Ff::player(101, 142, 27, 2000, PlayerSquadStatus::NotYetSet, Ff::far_contract());
        let better_b =
            Ff::player(102, 138, 25, 2000, PlayerSquadStatus::NotYetSet, Ff::far_contract());

        let mut country = Ff::country(vec![target, better_a, better_b]);

        let offer = TransferOffer::new(CurrencyValue::new(340_000.0, Currency::Usd), Ff::BUYER_ID, Ff::date());
        let neg = TransferNegotiation::new(
            Ff::NEG_ID, 100, 0, Ff::SELLER_ID, Ff::BUYER_ID, offer, Ff::date(), 0.75, 0.40, 26, 0.5,
        );
        country.transfer_market.negotiations.insert(Ff::NEG_ID, neg);

        // Synthetic-style asking (offer × 1.2) — the exact laundering that
        // made the bug's ratio look healthy. The floor ignores it.
        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);
        CountryResult::resolve_initial_approach(&mut country, Ff::NEG_ID, &nd, Ff::date());

        let neg = &country.transfer_market.negotiations[&Ff::NEG_ID];
        assert_eq!(
            neg.status,
            NegotiationStatus::Rejected,
            "a 340K bid for a valued first-team player must be rejected"
        );
        assert_eq!(
            neg.rejection_reason,
            Some(NegotiationRejectionReason::AskingPriceTooHigh),
            "the rejection must come from the market-value fee floor"
        );
    }

    /// Regression #3: an explicit `NotNeeded` surplus tag is a strong
    /// distressed reason — the importance premium is waived — but a valuable
    /// player still keeps the low distressed residual floor, so a 340K bid is
    /// rejected (the seller would release him free before giving him away).
    #[test]
    fn not_needed_player_keeps_distressed_residual_floor() {
        let target =
            Ff::player(100, 150, 26, 5000, PlayerSquadStatus::NotNeeded, Ff::far_contract());
        let country = Ff::country(vec![target]);
        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);

        let v = SellerFeeFloor::for_permanent_domestic(&country, &nd, Ff::date())
            .expect("a valuable NotNeeded player keeps a distressed residual floor");
        assert_eq!(v.distress, SellerDistress::Strong);
        assert_eq!(v.fraction, SellerFeeFloor::DISTRESSED_RESIDUAL);
        assert!(
            340_000.0 < v.min_fee,
            "a 340K bid ({}) is still below the distressed floor ({})",
            340_000.0,
            v.min_fee
        );
    }

    /// Regression #4: a transfer-listed first-team player gets only a MODEST
    /// discount — the floor stays at ~60% of market value, never a fraction
    /// of it. "Listed status alone" must not unlock a 340K sale.
    #[test]
    fn listed_first_team_player_keeps_modest_floor() {
        let mut target =
            Ff::player(100, 150, 26, 5000, PlayerSquadStatus::FirstTeamRegular, Ff::far_contract());
        target.statuses.add(Ff::date(), PlayerStatusType::Lst);
        let country = Ff::country(vec![target]);
        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);

        let v = SellerFeeFloor::for_permanent_domestic(&country, &nd, Ff::date())
            .expect("a listed first-team player still carries a floor");
        assert_eq!(v.asset_class, SquadAssetClass::FirstTeamUseful);
        assert_eq!(v.distress, SellerDistress::Modest);
        assert_eq!(
            v.fraction,
            SellerFeeFloor::FIRST_TEAM_FLOOR - SellerFeeFloor::MODEST_DISCOUNT
        );
        assert!(
            v.fraction >= 0.60,
            "a listed first-team floor must stay at/above 60% of value, got {}",
            v.fraction
        );
        assert!(340_000.0 < v.min_fee);
    }

    /// Regression #5: a genuine low-ability surplus player (well below the
    /// squad level, no distress) carries NO importance floor, so an
    /// around-value 340K sale can complete.
    #[test]
    fn genuine_low_value_surplus_has_no_floor() {
        // Four strong midfielders so the fringe man is clearly below the
        // squad average → inferred TrueSurplus (not via NotNeeded, so no
        // distressed residual either).
        let mut squad: Vec<Player> = (200..204)
            .map(|id| Ff::player(id, 140, 26, 1000, PlayerSquadStatus::NotYetSet, Ff::far_contract()))
            .collect();
        squad.push(Ff::player(
            100,
            95,
            33,
            500,
            PlayerSquadStatus::MainBackupPlayer,
            Ff::far_contract(),
        ));
        let country = Ff::country(squad);
        let nd = Ff::neg_data(100, 340_000.0, 400_000.0);

        assert!(
            SellerFeeFloor::for_permanent_domestic(&country, &nd, Ff::date()).is_none(),
            "a genuine low-value surplus player must carry no importance floor"
        );
    }

    /// Regression #6: a contract running down (<6 months) is a strong
    /// distressed reason — the floor collapses to the distressed residual, so
    /// a meaningfully lower fee can complete. Documents the allowed discount.
    #[test]
    fn near_expiry_contract_collapses_floor_to_residual() {
        let target = Ff::player(
            100,
            150,
            26,
            5000,
            PlayerSquadStatus::FirstTeamRegular,
            Ff::d(2026, 11, 1), // ~4 months from the test date
        );
        let country = Ff::country(vec![target]);
        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);

        let v = SellerFeeFloor::for_permanent_domestic(&country, &nd, Ff::date())
            .expect("a near-expiry first-team player keeps the residual floor");
        assert_eq!(v.distress, SellerDistress::Strong);
        assert_eq!(v.fraction, SellerFeeFloor::DISTRESSED_RESIDUAL);
        // The allowed discount: the floor is well below the healthy 70%
        // first-team floor — a near-expiry player genuinely goes cheaper.
        assert!(v.fraction < SellerFeeFloor::FIRST_TEAM_FLOOR);
    }

    /// Regression #8: the stale-listing decay floors an asking price at
    /// 0.6 × original. A core player's fee floor (0.8 × market value) sits
    /// ABOVE that deepest decay, so even a bid at the fully-decayed asking
    /// can't prise him away — the floor tracks his value, not the listing.
    #[test]
    fn decay_cannot_breach_important_player_floor() {
        let target =
            Ff::player(100, 150, 26, 5000, PlayerSquadStatus::KeyPlayer, Ff::far_contract());
        let country = Ff::country(vec![target]);
        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);

        let v = SellerFeeFloor::for_permanent_domestic(&country, &nd, Ff::date()).unwrap();
        let deepest_decay_bid = v.market_value * 0.6; // market.rs decay floor
        assert!(
            deepest_decay_bid < v.min_fee,
            "a bid at the deepest listing decay ({}) must still fall below the \
             core fee floor ({})",
            deepest_decay_bid,
            v.min_fee
        );
    }
}
