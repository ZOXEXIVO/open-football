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
use crate::country::result::CountryResult;
use crate::transfers::NegotiationStatus;
use crate::transfers::TransferListingStatus;
use crate::transfers::TransferRoutePolicy;
use crate::transfers::TransferWindowManager;
use crate::transfers::market::TransferListingOrigin;
use crate::transfers::negotiation::{NegotiationPhase, NegotiationRejectionReason, TransferNegotiation};
use crate::transfers::offer::TransferClause;
use crate::transfers::pipeline::PipelineProcessor;
use crate::transfers::pipeline::plausibility::{
    TransferPlausibilityBuilder, TransferPlausibilityEvaluator, TransferPlausibilityVerdict,
};
use crate::transfers::scouting_region::ScoutingRegion;
use crate::utils::{FloatUtils, FormattingUtils};
use crate::{
    Country, PlayerSquadStatus, PlayerStatusType, TransferInterestSource, TransferInterestStage,
    WageCalculator,
};
use chrono::NaiveDate;

impl CountryResult {
    pub(crate) fn resolve_pending_negotiations(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) -> Vec<DeferredTransfer> {
        let mut deferred: Vec<DeferredTransfer> = Vec::new();
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
                    Self::resolve_personal_terms(country, neg_id, &neg_data, date);
                }
                NegotiationPhase::MedicalAndFinalization { .. } => {
                    Self::resolve_medical(
                        country,
                        country_id,
                        neg_id,
                        &neg_data,
                        date,
                        summary,
                        &mut deferred,
                    );
                }
            }
        }

        deferred
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
            if let Some(player) = find_player_in_country(country, neg_data.player_id) {
                if player.is_transfer_protected(date, current_window) {
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
            0.55
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
                let target = if neg_data.asking_price > 0.0 {
                    neg_data.asking_price * seller_reservation
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
    ) {
        let is_foreign = neg_data.selling_country_id.is_some();

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
        deferred: &mut Vec<DeferredTransfer>,
    ) {
        let is_foreign = neg_data.selling_country_id.is_some();

        // Cross-country route policy: the plausibility evaluator does not
        // run on cross-country negotiations (it operates inside one
        // country's clubs), so a Russia ↔ Ukraine bid that survived the
        // earlier scouting/shortlist filters can still arrive here. Refuse
        // it before the medical roll so a stale negotiation, restored save,
        // or alternate creation path can't complete a closed route.
        if is_foreign
            && TransferRoutePolicy::is_blocked(
                &neg_data.selling_country_code,
                &country.code,
                date,
            )
        {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(
                    NegotiationRejectionReason::CountryPairRouteBlocked,
                );
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
            if deferred.iter().any(|d| d.player_id == neg_data.player_id) {
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
                deferred.push(DeferredTransfer {
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
        let buyer_rep_factor = 1.0 + (negotiation.buying_club_reputation as f64).clamp(0.0, 1.0) * 0.15;

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
        let mut n = TransferNegotiation::new(1, 10, 0, 1, 2, offer, d(2026, 7, 1), 0.5, 0.5, 24, 0.5);
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
