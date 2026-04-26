use super::types::{
    find_player_in_country, find_player_in_country_mut, DeferredTransfer, NegotiationData,
    TransferActivitySummary,
};
use crate::club::player::agent::PlayerAgent;
use crate::country::result::CountryResult;
use crate::transfers::negotiation::{NegotiationPhase, NegotiationRejectionReason};
use crate::transfers::offer::TransferClause;
use crate::transfers::pipeline::PipelineProcessor;
use crate::transfers::scouting_region::ScoutingRegion;
use crate::transfers::TransferListingStatus;
use crate::transfers::TransferWindowManager;
use crate::utils::{FloatUtils, FormattingUtils};
use crate::club::player::calculators::{
    squad_status_wage_factor, ContractValuation, ValuationContext,
};
use crate::{Country, PlayerSquadStatus, PlayerStatusType, WageCalculator};
use chrono::NaiveDate;

impl CountryResult {
    pub(crate) fn resolve_pending_negotiations(
        country: &mut Country,
        date: NaiveDate,
        summary: &mut TransferActivitySummary,
    ) -> Vec<DeferredTransfer> {
        let mut deferred: Vec<DeferredTransfer> = Vec::new();
        let country_id = country.id;

        let ready_to_resolve: Vec<u32> = country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| n.is_phase_ready(date))
            .map(|n| n.id)
            .collect();

        for neg_id in ready_to_resolve {
            let neg_data = match country.transfer_market.negotiations.get(&neg_id) {
                Some(n) => {
                    let asking_price = country
                        .transfer_market
                        .listings
                        .get(n.listing_id as usize)
                        .map(|l| l.asking_price.amount)
                        .unwrap_or(0.0);
                    let is_listed = country
                        .transfer_market
                        .listings
                        .get(n.listing_id as usize)
                        .map(|l| {
                            l.status == TransferListingStatus::InNegotiation
                                || l.status == TransferListingStatus::Available
                        })
                        .unwrap_or(false);
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
                        is_listed,
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
            let window_mgr = TransferWindowManager::new();
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

        let ratio = if neg_data.asking_price > 0.0 {
            neg_data.offer_amount / neg_data.asking_price
        } else {
            1.0
        };

        let mut chance: f32 = if neg_data.is_listed {
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
            // Interest is now real — fire the tap-up happiness event on the
            // target so the rumour mill actually nudges morale. Only fires
            // for domestic targets; foreign players sit outside this country.
            if neg_data.selling_country_id.is_none() {
                let buyer_rep = neg_data.buying_rep;
                let seller_rep = neg_data.selling_rep;
                if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                    player.on_transfer_interest_confirmed(buyer_rep, seller_rep);
                }
            }
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation
                    .reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            // Domestic targets feel the rejection if it was a real chance —
            // buyer must be meaningfully bigger and either the player was
            // already pushing for a move or has the ambition to feel snubbed.
            // Favorite-club bid being rejected hurts even at lateral rep.
            if neg_data.selling_country_id.is_none() {
                let buyer_rep = neg_data.buying_rep;
                let seller_rep = neg_data.selling_rep;
                let buying_club_id = neg_data.buying_club_id;
                if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                    let was_favorite = player.favorite_clubs.contains(&buying_club_id);
                    player.on_transfer_bid_rejected(buyer_rep, seller_rep, was_favorite);
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
        let mut seller_reservation = if neg_data.is_listed { 0.82 } else { 1.08 };

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

        let urgency = Self::deadline_urgency(country.id, date) as f64;
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
            // and the selling club still said no. Same gating as the
            // initial-approach path, including favorite-club amplification.
            if neg_data.selling_country_id.is_none() {
                let buyer_rep = neg_data.buying_rep;
                let seller_rep = neg_data.selling_rep;
                let buying_club_id = neg_data.buying_club_id;
                if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                    let was_favorite = player.favorite_clubs.contains(&buying_club_id);
                    player.on_transfer_bid_rejected(buyer_rep, seller_rep, was_favorite);
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

        // Release clause: the player almost always welcomes the move they
        // negotiated the escape route for. Overrides downward-move and
        // salary resistance below.
        if !is_foreign && Self::clause_triggers_sale(country, neg_data) {
            chance += 45.0;
        }

        // End-of-window pressure: players prefer a signed deal over an
        // expired negotiation that drops them back into limbo.
        chance += Self::deadline_urgency(country.id, date) * 15.0;

        if neg_data.is_listed {
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
                deferred.push(DeferredTransfer {
                    player_id: neg_data.player_id,
                    selling_country_id,
                    selling_club_id: neg_data.selling_club_id,
                    buying_country_id: country_id,
                    buying_club_id: neg_data.buying_club_id,
                    fee: neg_data.offer_amount,
                    is_loan: neg_data.is_loan,
                    has_option_to_buy: neg_data.has_option_to_buy,
                    agreed_annual_wage: neg_data.offered_annual_wage,
                    buying_league_reputation: neg_data.buying_league_reputation,
                    sell_on_percentage: neg_data.sell_on_percentage,
                    loan_future_fee: neg_data.loan_future_fee,
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
            // and only the medical stood in the way. Strong morale event
            // for domestic players if the destination was meaningfully
            // bigger or a known favorite club.
            if neg_data.selling_country_id.is_none() {
                let buyer_rep = neg_data.buying_rep;
                let seller_rep = neg_data.selling_rep;
                let buying_club_id = neg_data.buying_club_id;
                if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                    let was_favorite = player.favorite_clubs.contains(&buying_club_id);
                    player.on_dream_move_collapsed(buyer_rep, seller_rep, was_favorite);
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
                && (n.status == crate::transfers::NegotiationStatus::Pending
                    || n.status == crate::transfers::NegotiationStatus::Countered)
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

    /// How close are we to the transfer window slamming shut? 0 when at
    /// least two weeks remain; ramps linearly to 1.0 on deadline day.
    /// Used to push both sides toward a deal instead of letting stale
    /// negotiations age into the expiry rejection.
    pub(crate) fn deadline_urgency(country_id: u32, date: NaiveDate) -> f32 {
        let mgr = TransferWindowManager::new();
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
