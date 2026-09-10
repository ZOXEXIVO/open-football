//! The cross-border half of opening a negotiation.
//!
//! Three passes, which the comments inside the old 894-line
//! `initiate_foreign_negotiations` already named: read every buying club's
//! shortlist for names that are not in this country, resolve each one against
//! the world and decide the offer, then write the listings and negotiations.
//!
//! The offer itself is **not** here. It is [`super::ApproachBuilder`], shared
//! with the domestic pass — this file used to carry a copy of it, with a dozen
//! comments saying "mirror the domestic path", which is exactly how the two
//! drifted apart. What is genuinely cross-border stays: resolving the player's
//! club across the world index, the foreigner-registration gate, and the
//! seller-side facts staged here because the buying country's resolver cannot
//! reach back over the border to recompute them.

use chrono::NaiveDate;
use log::debug;
use std::sync::Arc;

use crate::SimulatorData;
use crate::shared::CurrencyValue;
use crate::transfers::MarketMap;
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::{Club, Country, Player};

use super::*;

/// A name on some club's shortlist that is not in this country.
struct ForeignCandidate {
    buying_club_id: u32,
    player_id: u32,
    shortlist_request_id: u32,
}

/// A cross-border approach, decided and waiting to be written. Everything the
/// shared builder produced, plus the seller-side facts only this side of the
/// border can see.
struct ResolvedNeg {
    buying_club_id: u32,
    selling_country_id: u32,
    selling_continent_id: u32,
    selling_country_code: String,
    selling_club_id: u32,
    player_id: u32,
    is_loan: bool,
    has_option_to_buy: bool,
    is_prospect_purchase: bool,
    offer: TransferOffer,
    reason: TransferReason,
    shortlist_request_id: u32,
    selling_rep: f32,
    buying_rep: f32,
    player_age: u8,
    player_ambition: f32,
    asking_price: CurrencyValue,
    player_name: String,
    selling_club_name: String,
    player_sold_from: Option<(u32, f64)>,
    offered_annual_wage: u32,
    buying_league_reputation: u16,
    /// The SELLER's league reputation — see the domestic action.
    selling_league_reputation: u16,
    /// The player's big-stage pull, staged for the resolver.
    player_stage_inclination: f32,
    /// Cold cross-border approach (target not seller-advertised).
    /// Stamped on the negotiation so the resolver applies the
    /// unsolicited base chance — the foreign path used to leave
    /// the flag unset and cold calls abroad engaged at the easier
    /// solicited baseline.
    is_unsolicited: bool,
    /// Captured at creation from the full cross-border assessment:
    /// the player would refuse this move on willingness grounds
    /// (a clear step down with no availability signal). Applied as
    /// the foreign personal-terms hard floor — the buyer's country
    /// no longer holds the seller-side data to recompute it.
    foreign_terms_floor_blocked: bool,
    /// Seller-side player importance captured at creation (same 0..1
    /// scale as the domestic resolver computes). Rides into the
    /// foreign club-fee resolver so a foreign deal faces the same
    /// importance-driven seller reservation as a domestic one,
    /// instead of a flat mid-range constant.
    foreign_seller_importance: f32,
    /// The buyer's own ceiling for this deal and the tier of the
    /// request it answers — see the domestic action.
    buyer_ceiling_fee: Option<f64>,
    brief_tier: Option<BriefTier>,
    /// Selling club's `(annual income, wage bill, wage budget)` — see
    /// the staged field on the negotiation.
    foreign_seller_finances: (i64, i64, i64),
    /// The player's own side of the appraisal, captured here
    /// because this is the last moment his country is in scope.
    /// See [`crate::transfers::gate::appraisal::PlayerStance`].
    staged_stance: PlayerStance,
    /// Sporting distance of the move — needs both clubs, so it is
    /// read here rather than guessed at resolution.
    staged_sporting_drop: f32,
}

/// The pass itself: read the shortlists, resolve each name, write the result.
pub(in crate::transfers::pipeline) struct ForeignApproachPass;

impl ForeignApproachPass {
    pub(in crate::transfers::pipeline) fn run(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        let candidates = Self::candidates(data, country_id);
        if candidates.is_empty() {
            return;
        }

        let mut resolved: Vec<ResolvedNeg> = Vec::new();
        // Foreign candidates the final cross-border gate refuses — marked
        // on the shortlist so the cursor advances instead of stalling.
        let mut foreign_rejected: Vec<PlausibilityReject> = Vec::new();
        // The world map, taken once outside the loop: the geography gate
        // reads it per candidate.
        //
        // Cloned rather than borrowed because the resolve pass below needs
        // `data` for the country lookups.
        let market_map = Arc::clone(&data.market_map);
        // Foreigner-quota room per buying club, counted lazily and once.
        let mut foreign_registration = ForeignRegistrationGuard::default();

        for cand in candidates {
            match Self::resolve(
                data,
                &cand,
                &mut foreign_registration,
                &ForeignTick {
                    country_id,
                    date,
                    market_map: market_map.as_ref(),
                },
            ) {
                Resolution::Approach(neg) => resolved.push(*neg),
                Resolution::Refused(reject) => foreign_rejected.push(reject),
                Resolution::Skip => {}
            }
        }

        Self::commit(data, country_id, date, resolved);
        Self::apply_rejects(data, country_id, foreign_rejected);
    }

    /// Pass 1 — read: every shortlist candidate this country does not hold.
    fn candidates(data: &SimulatorData, country_id: u32) -> Vec<ForeignCandidate> {
        let mut candidates: Vec<ForeignCandidate> = Vec::new();

        if let Some(country) = data.country(country_id) {
            for club in &country.clubs {
                let plan = &club.transfer_plan;
                if !plan.initialized || !plan.can_start_negotiation() {
                    continue;
                }

                // Same squad-cap gate as the domestic path: a club whose
                // Main roster is full keeps agreeing cross-border deals the
                // executor then refuses, holding slots and budget for the
                // whole multi-phase lifetime each time.
                if !ClubView::can_accept_player(club) {
                    continue;
                }

                let actual_active = country
                    .transfer_market
                    .active_negotiation_count_for_club(club.id);
                if actual_active >= plan.max_concurrent_negotiations {
                    continue;
                }

                // Per-tick slot budget, mirroring the domestic pass — the
                // one-shot cap check above let a club with N shortlists
                // open N foreign negotiations in a single tick, blowing
                // past `max_concurrent_negotiations`.
                let slots_available = plan
                    .max_concurrent_negotiations
                    .saturating_sub(actual_active) as usize;
                let mut negotiations_this_club = 0usize;

                for shortlist in &plan.shortlists {
                    if negotiations_this_club >= slots_available {
                        break;
                    }
                    if shortlist.has_pursuing_candidate() || shortlist.all_exhausted() {
                        continue;
                    }

                    // Mirror the domestic request-liveness gate: vetoed
                    // (board_approved == false / Abandoned) and Fulfilled
                    // requests must not be pursued abroad either.
                    let request_live = plan
                        .transfer_requests
                        .iter()
                        .find(|r| r.id == shortlist.transfer_request_id)
                        .map(|r| {
                            r.status != TransferRequestStatus::Abandoned
                                && r.status != TransferRequestStatus::Fulfilled
                                && r.board_approved != Some(false)
                        })
                        .unwrap_or(true);
                    if !request_live {
                        continue;
                    }

                    let candidate = match shortlist.current_candidate() {
                        Some(c) if c.status == ShortlistCandidateStatus::Available => c,
                        _ => continue,
                    };

                    // Only process if player is NOT in the local country
                    let is_local =
                        PipelineProcessor::find_player_in_country(country, candidate.player_id)
                            .is_some();
                    if is_local {
                        continue;
                    }

                    if country
                        .transfer_market
                        .has_active_negotiation_for(candidate.player_id, club.id)
                    {
                        continue;
                    }

                    candidates.push(ForeignCandidate {
                        buying_club_id: club.id,
                        player_id: candidate.player_id,
                        shortlist_request_id: shortlist.transfer_request_id,
                    });
                    negotiations_this_club += 1;
                }
            }
        }

        candidates
    }

    /// Pass 2 — resolve one candidate: read both sides, gate the move, and
    /// stage the seller-side facts the buyer's own country cannot recompute.
    fn resolve(
        data: &SimulatorData,
        cand: &ForeignCandidate,
        foreign_registration: &mut ForeignRegistrationGuard,
        tick: &ForeignTick<'_>,
    ) -> Resolution {
        let date = tick.date;
        let market_map = tick.market_map;

        let Some(seller) = Self::seller(data, cand.player_id, tick) else {
            return Resolution::Skip;
        };
        let Some(buyer) = Self::buyer(data, cand, tick, &seller) else {
            return Resolution::Skip;
        };

        // Read both sides back into the names the gates below already used.
        // References, not moves: `seller` and `buyer` are handed whole to the
        // offer builder afterwards.
        let sell_country = seller.country;
        let sell_club = seller.club;
        let player = seller.player;
        let player_name = &seller.player_name;
        let selling_club_name = &seller.club_name;
        let selling_rep = seller.rep;
        let asking_price = &seller.asking_price;
        let buy_country = buyer.country;
        let buy_club = buyer.club;
        let buying_rep = buyer.rep;
        let budget = buyer.budget;
        let is_loan = buyer.is_loan;

        // ── Registration gate ────────────────────────────────────────
        // Counted per (club, group) and memoised for this pass: this is
        // the one buy path that reached `from_global` with no squad-fit
        // snapshot at all, so a club at its foreigner quota could open
        // talks for a foreigner it could never register. Its candidates
        // are normally gated upstream by the shortlist, but a
        // `KnownPlayerMemory` or staff-recommendation candidate arrives
        // without one.
        //
        // Read as a gate, not a preference: the registration rule is
        // TRUTH about the buyer, and truth is what gates read.
        if foreign_registration.would_block(buy_country, cand.buying_club_id, player.country_id) {
            debug!(
                "Foreign negotiation suppressed: club {} has no registration slot for {} ({})",
                cand.buying_club_id, cand.player_id, player_name
            );
            return Resolution::Refused(PlausibilityReject {
                club_id: cand.buying_club_id,
                player_id: cand.player_id,
                shortlist_request_id: cand.shortlist_request_id,
            });
        }

        // ── Final foreign plausibility gate ──────────────────────────
        // Mirror the domestic gate in `initiate_negotiations`: before
        // fabricating a SyntheticUnsolicited listing or opening talks,
        // assess the FULL cross-border move with both clubs/countries in
        // hand. A lower-league side abroad chasing an important
        // first-teamer at a much stronger club cannot credibly reach
        // negotiation — refuse it here so no synthetic listing is created
        // and the shortlist advances past the dud. This is the gate that
        // stops Sambenedettese opening talks for a Spartak first-teamer.
        let plausibility_inputs = TransferPlausibilityBuilder::from_global(
            buy_country,
            buy_club,
            sell_country,
            sell_club,
            player,
            asking_price.amount,
            is_loan,
            true, // unsolicited — the buyer is reaching out abroad
            date,
            &market_map,
        );
        let assessment = TransferMovePlausibility::assess(&plausibility_inputs);
        if TransferTrace::is(cand.player_id) {
            TransferTrace::line(
                cand.player_id,
                "plaus",
                format!(
                    "buyer={} ({}) seller={} stage={:?} asking={:.0} budget={:.0} \
             agent_channel={} — {}",
                    buy_club.name,
                    buy_club.id,
                    selling_club_name,
                    assessment.stage,
                    asking_price.amount,
                    budget,
                    plausibility_inputs.is_agent_circulated(),
                    assessment.diagnostics.explain(),
                ),
            );
        }
        if !assessment.reaches(TransferMoveStage::CanStartNegotiation) {
            debug!(
                "Foreign negotiation suppressed: club {} won't pursue {} ({}) from {} — {}",
                cand.buying_club_id,
                cand.player_id,
                player_name,
                selling_club_name,
                assessment.diagnostics.explain()
            );
            return Resolution::Refused(PlausibilityReject {
                club_id: cand.buying_club_id,
                player_id: cand.player_id,
                shortlist_request_id: cand.shortlist_request_id,
            });
        }

        // Personal-terms willingness floor, captured now (full seller
        // context in scope) for application at the PersonalTerms phase —
        // the buyer's country won't hold the seller-side data then.
        let foreign_terms_floor_blocked =
            TransferMovePlausibility::player_terms_floor(&plausibility_inputs).is_some();
        // Capture seller-side importance now (full cross-border context
        // in scope) so the foreign club-fee resolver applies the same
        // importance-driven reservation a domestic seller would, instead
        // of a flat constant that made foreign buys too easy. The
        // assessment already derived it from the seller's squad-status
        // and position rank.
        let foreign_seller_importance = assessment.diagnostics.importance;
        // The seller's books, staged now: at resolution time this club
        // sits inside another country's borrow, so the windfall model
        // could not otherwise ask what the fee is worth to him.
        // The player's own side of the decision, captured while his
        // country is still in scope. Personal terms are resolved by the
        // BUYING country's pass, where he is unreachable — so without
        // this the cross-border path could only fall back to a bare
        // prestige wall, which is precisely what refused every money
        // move before the money was looked at (L1).
        let staged_sporting_drop =
            TransferPlausibilityEvaluator::sporting_drop(&plausibility_inputs);
        // No listing is bound to a cross-border approach — the buyer
        // reads his badges, not a row in its own market.
        let availability = AvailabilityView::read(player, is_loan, None);
        let staged_stance = PlayerStanceBuilder::build(&StanceInputs {
            player,
            seller_country: sell_country,
            seller_club: sell_club,
            buyer_club_id: cand.buying_club_id,
            rep_diff: buying_rep - selling_rep,
            importance: foreign_seller_importance,
            // The ONE availability reading, type-matched to the deal
            // on the table ([`AvailabilityView`]): a loan-listed man
            // was collecting the seller's-advert push toward a
            // PERMANENT move abroad and not toward the same move at
            // home.
            listed_by_club: availability.listed_by_club,
            available: availability.available_soft,
            months_to_tournament: sell_country
                .months_to_tournament_for(player.nationality_continent_id),
            date,
        });

        let foreign_seller_finances = (
            sell_club.finance.estimated_annual_income(date),
            sell_club
                .teams
                .iter()
                .map(|t| t.get_annual_salary() as i64)
                .sum::<i64>(),
            sell_club
                .board
                .season_targets
                .as_ref()
                .map(|t| t.wage_budget.max(0) as i64)
                .unwrap_or(0),
        );

        Self::offer(
            seller,
            buyer,
            StagedSellerFacts {
                terms_floor_blocked: foreign_terms_floor_blocked,
                seller_importance: foreign_seller_importance,
                seller_finances: foreign_seller_finances,
                stance: staged_stance,
                sporting_drop: staged_sporting_drop,
            },
            cand,
            tick,
        )
    }

    /// The buying side: who is asking, what it can spend, and — from the
    /// director of football — whether this is a buy, a loan, or a loan with
    /// an option.
    fn buyer<'a>(
        data: &'a SimulatorData,
        cand: &ForeignCandidate,
        tick: &ForeignTick<'_>,
        seller: &ForeignSeller<'_>,
    ) -> Option<ForeignBuyer<'a>> {
        let country_id = tick.country_id;
        let date = tick.date;
        let player = seller.player;
        let player_age = seller.player_age;
        let selling_rep = seller.rep;
        let asking_price = &seller.asking_price;

        let buy_country = match data.country(country_id) {
            Some(c) => c,
            None => return None,
        };
        let buy_club = match buy_country
            .clubs
            .iter()
            .find(|c| c.id == cand.buying_club_id)
        {
            Some(c) => c,
            None => return None,
        };

        let buying_rep = buy_club
            .teams
            .teams
            .first()
            .map(|t| t.reputation.world as f32 / 10000.0)
            .unwrap_or(0.3);
        let rep_level = buy_club
            .teams
            .teams
            .first()
            .map(|t| t.reputation.level())
            .unwrap_or(ReputationLevel::Amateur);
        let budget = buy_club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or_else(|| (buy_club.finance.balance.balance.max(0) as f64) * 0.3);

        let request = buy_club
            .transfer_plan
            .transfer_requests
            .iter()
            .find(|r| r.id == cand.shortlist_request_id);

        // Scout-side context for the foreign target. Monitoring rows
        // live with the buying club's plan; believed ability/potential
        // feeds the buy/loan decision and the offer strategy — hidden
        // PA is never consulted.
        let monitoring = buy_club
            .transfer_plan
            .scout_monitoring
            .iter()
            .find(|m| m.player_id == cand.player_id);
        let scouting_report = buy_club
            .transfer_plan
            .scouting_reports
            .iter()
            .find(|r| r.player_id == cand.player_id);
        let scout_assessed = monitoring
            .map(|m| (m.current_assessed_ability, m.current_assessed_potential))
            .or_else(|| scouting_report.map(|r| (r.assessed_ability, r.assessed_potential)));
        let scout_confidence = monitoring
            .map(|m| m.confidence)
            .or_else(|| scouting_report.map(|r| r.confidence));

        let buying_league_reputation = buy_club
            .teams
            .teams
            .first()
            .and_then(|t| t.league_id)
            .and_then(|lid| buy_country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        // Same "gettable" / wage-room signals as the domestic path.
        let target_available = player.statuses.has(PlayerStatusType::Lst)
            || player.statuses.has(PlayerStatusType::Loa)
            || player.statuses.has(PlayerStatusType::Req)
            || player.statuses.has(PlayerStatusType::Unh)
            || (player.statistics.played + player.statistics.played_subs) < 10;
        let committed_wages: f64 = buy_club
            .teams
            .iter()
            .map(|t| t.get_annual_salary() as f64)
            .sum();
        let wage_headroom = buy_club
            .board
            .season_targets
            .as_ref()
            .map(|t| (t.wage_budget.max(0) as f64 - committed_wages).max(0.0));
        let expected_wage = WageCalculator::expected_annual_wage(
            player,
            player_age,
            buying_rep,
            buying_league_reputation,
        );

        let prospect_ctx = ProspectSigningContext {
            scout_assessed,
            scout_confidence,
            prospect_slots_used: buy_club
                .transfer_plan
                .prospect_buys_this_window
                .saturating_add(buy_club.transfer_plan.prospect_pursuits_active),
            seller_rep_score: selling_rep,
            buyer_rep_score: buying_rep,
            target_available,
            wage_headroom,
            expected_wage,
        };

        let approach = PipelineProcessor::determine_transfer_approach(
            &rep_level,
            budget,
            asking_price.amount,
            request,
            player_age,
            date,
            buy_club.finance.balance.balance,
            &buy_club.philosophy,
            &prospect_ctx,
        );

        let is_loan = !matches!(approach, TransferApproach::PermanentTransfer);
        let has_option_to_buy = matches!(approach, TransferApproach::LoanWithOption);
        let is_prospect_purchase = !is_loan
            && matches!(
                request.map(|r| &r.reason),
                Some(TransferNeedReason::DevelopmentSigning)
            );

        Some(ForeignBuyer {
            country: buy_country,
            club: buy_club,
            rep: buying_rep,
            league_reputation: buying_league_reputation,
            budget,
            request,
            monitoring,
            scouting_report,
            approach,
            is_loan,
            has_option_to_buy,
            is_prospect_purchase,
        })
    }

    /// The offer, from the shared builder, plus the staging this reach adds.
    fn offer(
        seller: ForeignSeller<'_>,
        buyer: ForeignBuyer<'_>,
        staged: StagedSellerFacts,
        cand: &ForeignCandidate,
        tick: &ForeignTick<'_>,
    ) -> Resolution {
        let date = tick.date;
        let market_map = tick.market_map;
        let ForeignSeller {
            country: sell_country,
            club: sell_club,
            player,
            country_id: sell_country_id,
            club_id: sell_club_id,
            continent_id: sell_continent_id,
            country_code: sell_country_code,
            price_level: sell_price_level,
            asking_price,
            player_age,
            player_ambition,
            player_name,
            club_name: selling_club_name,
            rep: selling_rep,
            league_reputation: selling_league_reputation,
        } = seller;
        let ForeignBuyer {
            country: buy_country,
            club: buy_club,
            rep: buying_rep,
            league_reputation: buying_league_reputation,
            budget,
            request,
            monitoring,
            scouting_report,
            approach,
            is_loan,
            has_option_to_buy,
            is_prospect_purchase,
        } = buyer;
        let StagedSellerFacts {
            terms_floor_blocked: foreign_terms_floor_blocked,
            seller_importance: foreign_seller_importance,
            seller_finances: foreign_seller_finances,
            stance: staged_stance,
            sporting_drop: staged_sporting_drop,
        } = staged;

        let avg_ability: u8 = buy_club
            .teams
            .teams
            .first()
            .map(|t| {
                let avg = t.players.current_ability_avg();
                if avg == 0 { 50 } else { avg }
            })
            .unwrap_or(50);

        let buyer_valuation_rep = buy_club
            .teams
            .teams
            .first()
            .map(|t| t.reputation.market_value_score())
            .filter(|&s| s > 0)
            .unwrap_or_else(|| (avg_ability as u16).saturating_mul(100).min(10_000));

        // The offer itself. One implementation, shared with the domestic
        // pass: strategy, asking price, wage, deal valuation, the man he
        // would replace, the opening ratio, the clauses and the reason all
        // live in `ApproachBuilder::build` now. This pass used to carry a
        // copy of all of it, with a dozen comments saying "mirror the
        // domestic path" — which is exactly how the two drifted.
        //
        // What could not be shared is named in `ApproachDrift`. Those are
        // the places the two copies had already diverged; each is spelled
        // out here rather than quietly resolved by the merge, because each
        // one moves money and wants its own census run.
        let Some(buy_team) = buy_club.teams.teams.first() else {
            return Resolution::Skip;
        };
        let outcome = ApproachBuilder::build(
            &ApproachBuyer {
                club: buy_club,
                team: buy_team,
                plan: &buy_club.transfer_plan,
                rep_score: buying_rep,
                league_reputation: buying_league_reputation,
                avg_ability,
                budget,
            },
            &ApproachTarget {
                player,
                selling_club: sell_club,
                selling_club_id: sell_club_id,
                selling_rep_score: selling_rep,
                selling_league_reputation,
                // Rivalry is a domestic idea in this model; the
                // cross-border pass has never read it.
                is_rival: false,
                monitoring,
                scouting_report,
            },
            request,
            &ApproachContext {
                buy_country,
                sell_country,
                market_map,
                price_level: sell_price_level,
                date,
                shortlist_request_id: cand.shortlist_request_id,
                approach: approach.clone(),
                is_loan,
                has_option_to_buy,
                is_prospect_purchase,
            },
            &ApproachDrift {
                allocated_budget: budget,
                valuation_reputation: buyer_valuation_rep,
                shortlist_rank: None,
                competition_count: None,
                loan_appearance_fee: false,
                generic_reason_fallback: true,
                // Already gated, harder, above — that assessment is where
                // the staged seller-side facts came from.
                final_gate_fee: None,
            },
        );
        let ApproachOutcome::Approach(action) = outcome else {
            return Resolution::Skip;
        };
        let action = *action;

        // A seller-advertised player (transfer- or loan-listed) makes
        // this a solicited approach; anyone else is a cold call. The
        // domestic path derives the same flag from the listing table;
        // the player's own status flags are the cross-border proxy.
        let is_unsolicited = !player.statuses.has(PlayerStatusType::Lst)
            && !player.statuses.has(PlayerStatusType::Loa);

        let neg = ResolvedNeg {
            buying_club_id: cand.buying_club_id,
            selling_country_id: sell_country_id,
            selling_continent_id: sell_continent_id,
            selling_country_code: sell_country_code,
            selling_club_id: sell_club_id,
            player_id: cand.player_id,
            is_loan,
            has_option_to_buy,
            is_prospect_purchase,
            offer: action.offer,
            reason: action.reason,
            shortlist_request_id: cand.shortlist_request_id,
            selling_rep,
            buying_rep,
            player_age,
            player_ambition,
            asking_price,
            player_name,
            selling_club_name,
            player_sold_from: player.sold_from.clone(),
            offered_annual_wage: action.offered_annual_wage,
            buying_league_reputation,
            selling_league_reputation,
            player_stage_inclination: player.big_stage_inclination,
            buyer_ceiling_fee: action.buyer_ceiling_fee,
            brief_tier: action.brief_tier,
            is_unsolicited,
            foreign_terms_floor_blocked,
            foreign_seller_importance,
            foreign_seller_finances,
            staged_stance,
            staged_sporting_drop,
        };

        Resolution::Approach(Box::new(neg))
    }

    /// The selling side, read while his country is still in scope.
    fn seller<'a>(
        data: &'a SimulatorData,
        player_id: u32,
        tick: &ForeignTick<'_>,
    ) -> Option<ForeignSeller<'a>> {
        let country_id = tick.country_id;
        let date = tick.date;
        // Resolve the player's current foreign club via the O(1)
        // global index (verified, with a full-scan fallback for a
        // stale entry) instead of re-walking the whole world per
        // candidate.
        let found = PipelineProcessor::resolve_foreign_player_club(data, country_id, player_id);

        let (sell_country_id, sell_club_id, sell_price_level, sell_continent_id, sell_country_code) =
            match found {
                Some(v) => v,
                None => return None,
            };

        let sell_country = match data.country(sell_country_id) {
            Some(c) => c,
            None => return None,
        };
        let player = match PipelineProcessor::find_player_in_country(sell_country, player_id) {
            Some(p) => p,
            None => return None,
        };
        if player.is_on_loan() {
            return None;
        }
        // Use the selling-side country reference (already in scope as
        // `sell_country`) so its country-specific calendar is honoured
        // — the buyer-side window doesn't apply when the player sits
        // in a different country's market.
        let sell_window = TransferWindowManager::for_country(sell_country, date)
            .current_window_dates(sell_country_id, date);
        if player.is_transfer_protected(date, sell_window) {
            return None;
        }

        let sell_club = match sell_country.clubs.iter().find(|c| c.id == sell_club_id) {
            Some(c) => c,
            None => return None,
        };
        let asking_price = PipelineProcessor::calculate_asking_price(
            player,
            sell_country,
            sell_club,
            date,
            sell_price_level,
        );
        let player_age = player.age(date);
        let player_ambition = player.skills.mental.determination;
        let player_name = player.full_name.to_string();
        let selling_club_name = sell_club.name.clone();

        let selling_rep = sell_club
            .teams
            .teams
            .first()
            .map(|t| t.reputation.world as f32 / 10000.0)
            .unwrap_or(0.3);
        let selling_league_reputation = sell_club
            .teams
            .teams
            .first()
            .and_then(|t| t.league_id)
            .and_then(|lid| sell_country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        Some(ForeignSeller {
            country: sell_country,
            club: sell_club,
            player,
            country_id: sell_country_id,
            club_id: sell_club_id,
            continent_id: sell_continent_id,
            country_code: sell_country_code,
            price_level: sell_price_level,
            asking_price,
            player_age,
            player_ambition,
            player_name,
            club_name: selling_club_name,
            rep: selling_rep,
            league_reputation: selling_league_reputation,
        })
    }

    /// Pass 3 — write: create the listings and open the negotiations.
    fn commit(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
        resolved: Vec<ResolvedNeg>,
    ) {
        // Pass 3: Write — create listings and negotiations
        for action in resolved {
            let country = match data.country_mut(country_id) {
                Some(c) => c,
                None => continue,
            };

            let listing = TransferListing::new_with_origin(
                action.player_id,
                action.selling_club_id,
                0,
                action.asking_price,
                date,
                if action.is_loan {
                    TransferListingType::Loan
                } else {
                    TransferListingType::Transfer
                },
                TransferListingOrigin::SyntheticUnsolicited,
            );
            country.transfer_market.add_listing(listing);

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.buying_club_id,
                action.offer,
                date,
                action.selling_rep,
                action.buying_rep,
                action.player_age,
                action.player_ambition,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = action.is_loan;
                    negotiation.has_option_to_buy = action.has_option_to_buy;
                    negotiation.is_unsolicited = action.is_unsolicited;
                    negotiation.reason = action.reason;
                    negotiation.selling_country_id = Some(action.selling_country_id);
                    negotiation.selling_continent_id = Some(action.selling_continent_id);
                    negotiation.selling_country_code = action.selling_country_code;
                    negotiation.player_sold_from = action.player_sold_from;
                    negotiation.player_name = action.player_name;
                    negotiation.selling_club_name = action.selling_club_name;
                    negotiation.open_salary_at(action.offered_annual_wage);
                    // The player's resolution-time reservation for a
                    // cross-border move: his expected wage at the buyer
                    // plus a ~10% relocation premium. The opening offer
                    // sits below it on purpose — that gap is what the
                    // personal-terms wage rounds close when the deal
                    // stalls on money instead of dying outright.
                    negotiation.staged_reservation_wage =
                        Some(((action.offered_annual_wage as f64) * 1.10) as u32);
                    negotiation.buying_league_reputation = action.buying_league_reputation;
                    negotiation.selling_league_reputation = action.selling_league_reputation;
                    negotiation.player_stage_inclination = action.player_stage_inclination;
                    negotiation.foreign_terms_floor_blocked = action.foreign_terms_floor_blocked;
                    negotiation.foreign_seller_importance = Some(action.foreign_seller_importance);
                    negotiation.foreign_seller_finances = Some(action.foreign_seller_finances);
                    negotiation.staged_stance = Some(action.staged_stance);
                    negotiation.staged_sporting_drop = Some(action.staged_sporting_drop);
                    negotiation.buyer_ceiling_fee = action.buyer_ceiling_fee;
                    negotiation.brief_tier = action.brief_tier;
                }

                if let Some(club) = country
                    .clubs
                    .iter_mut()
                    .find(|c| c.id == action.buying_club_id)
                {
                    let plan = &mut club.transfer_plan;
                    if let Some(shortlist) = plan
                        .shortlists
                        .iter_mut()
                        .find(|s| s.transfer_request_id == action.shortlist_request_id)
                    {
                        if let Some(candidate) = shortlist.current_candidate_mut() {
                            if candidate.player_id == action.player_id {
                                candidate.status = ShortlistCandidateStatus::CurrentlyPursuing;
                            }
                        }
                    }
                    if let Some(req) = plan
                        .transfer_requests
                        .iter_mut()
                        .find(|r| r.id == action.shortlist_request_id)
                    {
                        req.status = TransferRequestStatus::Negotiating;
                    }
                    plan.active_negotiation_count += 1;
                    if action.is_prospect_purchase {
                        // Pursuit slot taken; converted into a completed
                        // buy (or released) in on_negotiation_resolved.
                        plan.prospect_pursuits_active =
                            plan.prospect_pursuits_active.saturating_add(1);
                    }
                }

                debug!(
                    "Foreign negotiation: Club {} started negotiation for player {} from country {}",
                    action.buying_club_id, action.player_id, action.selling_country_id
                );
            }
        }
    }

    /// Mark every refused candidate so its shortlist cursor advances.
    fn apply_rejects(
        data: &mut SimulatorData,
        country_id: u32,
        foreign_rejected: Vec<PlausibilityReject>,
    ) {
        // Apply the foreign plausibility rejects: mark each shortlist
        // candidate unavailable and advance the shortlist so the next
        // pursuit cycle skips the impossible move instead of retrying it.
        if !foreign_rejected.is_empty() {
            if let Some(country) = data.country_mut(country_id) {
                for reject in foreign_rejected {
                    if let Some(club) = country.clubs.iter_mut().find(|c| c.id == reject.club_id) {
                        if let Some(shortlist) = club
                            .transfer_plan
                            .shortlists
                            .iter_mut()
                            .find(|s| s.transfer_request_id == reject.shortlist_request_id)
                        {
                            if let Some(candidate) = shortlist
                                .candidates
                                .iter_mut()
                                .find(|c| c.player_id == reject.player_id)
                            {
                                candidate.status = ShortlistCandidateStatus::Unavailable;
                            }
                            shortlist.advance_to_next();
                        }
                    }
                    PipelineProcessor::on_negotiation_resolved(
                        country,
                        reject.club_id,
                        reject.player_id,
                        false,
                    );
                }
            }
        }
    }
}

/// The tick a cross-border approach is decided against.
struct ForeignTick<'a> {
    country_id: u32,
    date: NaiveDate,
    market_map: &'a MarketMap,
}

/// The buying side of a cross-border move.
struct ForeignBuyer<'a> {
    country: &'a Country,
    club: &'a Club,
    rep: f32,
    league_reputation: u16,
    budget: f64,
    request: Option<&'a TransferRequest>,
    monitoring: Option<&'a ScoutPlayerMonitoring>,
    scouting_report: Option<&'a DetailedScoutingReport>,
    /// Buy, loan, or loan-with-option — the director of football's call.
    approach: TransferApproach,
    is_loan: bool,
    has_option_to_buy: bool,
    is_prospect_purchase: bool,
}

/// What only the seller's side of the border can see, captured while his
/// country is still in scope. The buying country's resolver runs the rest of
/// the negotiation with no way to reach back for any of it — which is why
/// these exist at all, and why retiring them is its own step.
struct StagedSellerFacts {
    terms_floor_blocked: bool,
    seller_importance: f32,
    seller_finances: (i64, i64, i64),
    stance: PlayerStance,
    sporting_drop: f32,
}

/// The selling side of a cross-border move, read while his country is in scope.
struct ForeignSeller<'a> {
    country: &'a Country,
    club: &'a Club,
    player: &'a Player,
    country_id: u32,
    club_id: u32,
    continent_id: u32,
    country_code: String,
    price_level: f32,
    asking_price: CurrencyValue,
    player_age: u8,
    player_ambition: f32,
    player_name: String,
    club_name: String,
    rep: f32,
    league_reputation: u16,
}

/// What resolving one candidate produced.
enum Resolution {
    Approach(Box<ResolvedNeg>),
    /// The staged cross-border model refused: mark the shortlist candidate.
    Refused(PlausibilityReject),
    /// Nothing to mark — the player moved, is protected, or could not be read.
    Skip,
}
