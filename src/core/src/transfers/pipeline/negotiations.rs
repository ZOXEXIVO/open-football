use chrono::NaiveDate;
use log::debug;
use rayon::prelude::*;
use rustc_hash::FxHashSet;
use std::collections::{HashMap, HashSet};

use crate::SimulatorData;
use crate::club::player::transfer::FreeAgentBlockReason;
use crate::country::result::transfers::FreeAgentBumpBatch;
use crate::country::result::transfers::types::can_club_accept_player;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::TransferWindowManager;
use crate::transfers::market::{
    TransferListing, TransferListingOrigin, TransferListingStatus, TransferListingType,
};
use crate::transfers::negotiation::NegotiationStatus;
use crate::transfers::offer::{TransferClause, TransferOffer};
use crate::transfers::pipeline::ScoutMonitoringStatus;
use crate::transfers::pipeline::plausibility::{
    TransferMovePlausibility, TransferMoveStage, TransferPlausibilityBuilder,
    TransferPlausibilityEvaluator, TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::processor::PipelineProcessor;
use crate::transfers::pipeline::{
    ShortlistCandidateStatus, TransferApproach, TransferNeedPriority, TransferNeedReason,
    TransferRequest, TransferRequestStatus,
};
use crate::utils::FormattingUtils;
use crate::{
    ClubPhilosophy, ClubTransferStrategy, Country, Person, PlayerStatusType, ReputationLevel,
    StaffPosition, TransferStrategyContext, WageCalculator,
};

/// Continuous buying aggressiveness from reputation ratio.
/// Replaces the old bucketed ReputationLevel cliff. A club's willingness to
/// offer close to asking scales smoothly with how established it is, and
/// how big it is relative to the seller — a small club overreaching for a
/// giant's player stays disciplined; a giant dealing with a small club can
/// push hard because they can wear the premium.
fn buying_aggressiveness_from_rep(buying_score: f32, selling_score: f32) -> f32 {
    let base = 0.30 + 0.55 * buying_score.clamp(0.0, 1.0);
    let ratio = if selling_score > 0.01 {
        (buying_score / selling_score).clamp(0.4, 2.0)
    } else {
        1.2
    };
    let ratio_adj = (ratio - 1.0) * 0.06;
    (base + ratio_adj).clamp(0.25, 0.90)
}

/// Buyer-side context for the prospect buy-vs-loan decision. Bundles the
/// scouts' read on the target, the hoarding-cap usage, seller-vs-buyer
/// standing, and wage room — everything observable; the hidden biological
/// PA never feeds this.
pub(in crate::transfers::pipeline) struct ProspectSigningContext {
    /// Scouts' believed (ability, potential) from monitoring rows or
    /// scouting reports. `None` = no dossier → no basis to commit a fee.
    pub scout_assessed: Option<(u8, u8)>,
    /// Confidence of that read (0..1). A one-look dossier doesn't
    /// justify buying a teenager.
    pub scout_confidence: Option<f32>,
    /// Window-cap usage: completed prospect buys + pursuits in flight.
    pub prospect_slots_used: u8,
    /// Seller / buyer reputation `overall_score`s (0..1).
    pub seller_rep_score: f32,
    pub buyer_rep_score: f32,
    /// Target is realistically gettable from a peer/bigger seller:
    /// listed, loan-listed, transfer-requested, unhappy, or barely
    /// playing. Smaller sellers don't need this escape hatch.
    pub target_available: bool,
    /// Annual wage headroom under the board's wage budget, when a
    /// mandate is set. `None` = no budget to respect.
    pub wage_headroom: Option<f64>,
    /// Expected annual wage of the target at the buyer.
    pub expected_wage: u32,
}

struct NegotiationAction {
    club_id: u32,
    player_id: u32,
    selling_club_id: u32,
    offer: TransferOffer,
    is_loan: bool,
    has_option_to_buy: bool,
    /// Permanent buy of a DevelopmentSigning target — counted against
    /// the per-window prospect-purchase cap when the negotiation opens.
    is_prospect_purchase: bool,
    shortlist_request_id: u32,
    negotiator_staff_id: Option<u32>,
    reason: String,
    player_name: String,
    selling_club_name: String,
    player_sold_from: Option<(u32, f64)>,
    offered_annual_wage: u32,
    buying_league_reputation: u16,
    is_rival: bool,
    /// The SELLER's own asking price for this player (seller-context
    /// valuation for a permanent move, loan fee for a loan). Captured so
    /// the synthetic listing created to back an unsolicited bid advertises
    /// what the seller would ask — never the buyer's budget-capped offer.
    seller_asking: CurrencyValue,
}

/// Asking price for the synthetic listing the pipeline fabricates so an
/// unsolicited approach has something to negotiate against.
///
/// The asking price MUST reflect the SELLER's valuation of the player —
/// never the buyer's (budget-capped) offer. Anchoring it on the offer let a
/// cash-poor buyer define the seller's price: a 5M core player whose only
/// suitor could bid 340K got a ~408K synthetic ask (offer × 1.2), and the
/// seller then "accepted" ~1:1 against that fabricated number, selling a
/// first-team player for a fraction of his worth. Anchoring on the seller's
/// own asking keeps the offer ÷ asking ratio honest so the reservation
/// guards in the negotiation resolver can do their job.
struct SyntheticListingPrice;

impl SyntheticListingPrice {
    fn for_unsolicited(seller_asking: &CurrencyValue) -> CurrencyValue {
        CurrencyValue {
            amount: FormattingUtils::round_fee(seller_asking.amount.max(0.0)),
            currency: seller_asking.currency.clone(),
        }
    }
}

/// Candidate culled by the plausibility gate immediately before
/// negotiation creation. Pass 2 marks the shortlist candidate as
/// unavailable so the next pursuit cycle skips it rather than retrying
/// the same impossible move.
struct PlausibilityReject {
    club_id: u32,
    player_id: u32,
    shortlist_request_id: u32,
}

impl PipelineProcessor {
    pub fn initiate_negotiations(country: &mut Country, date: NaiveDate) {
        let mut actions: Vec<NegotiationAction> = Vec::new();
        let mut plausibility_rejected: Vec<PlausibilityReject> = Vec::new();
        let price_level = country.settings.pricing.price_level;
        let window_mgr = TransferWindowManager::for_country(country, date);
        let current_window = window_mgr.current_window_dates(country.id, date);

        for club in &country.clubs {
            let plan = &club.transfer_plan;

            if !plan.initialized || !plan.can_start_negotiation() {
                continue;
            }

            // Skip clubs that have reached their squad cap. Use the same
            // `can_club_accept_player` predicate the executor enforces: it
            // resolves the Main team by TeamType, not `teams[0]`. The old
            // `teams.first()` count gated against whatever team happened to
            // sit first (often a reserve/B roster), so a club whose Main was
            // already full kept agreeing deals the executor then refused —
            // re-pursuing the same target every evaluation cycle.
            if !can_club_accept_player(club) {
                continue;
            }

            let actual_active = country
                .transfer_market
                .active_negotiation_count_for_club(club.id);
            if actual_active >= plan.max_concurrent_negotiations {
                continue;
            }

            let budget = club
                .finance
                .transfer_budget
                .as_ref()
                .map(|b| b.amount)
                .unwrap_or_else(|| (club.finance.balance.balance.max(0) as f64) * 0.3);

            if club.teams.teams.is_empty() {
                continue;
            }

            let team = &club.teams.teams[0];
            let rep_level = team.reputation.level();
            let buying_rep_score = team.reputation.overall_score();
            let buying_league_reputation = team
                .league_id
                .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
                .map(|l| l.reputation)
                .unwrap_or(0);

            let avg_ability = {
                let avg = team.players.current_ability_avg();
                if avg == 0 { 50 } else { avg }
            };

            // Board wage mandate headroom — annual wages committed across
            // all squads vs the season wage budget. None when no mandate
            // has been set (fresh worlds, test fixtures).
            let committed_wages: f64 = club
                .teams
                .iter()
                .map(|t| t.get_annual_salary() as f64)
                .sum();
            let wage_headroom = club
                .board
                .season_targets
                .as_ref()
                .map(|t| (t.wage_budget.max(0) as f64 - committed_wages).max(0.0));

            let slots_available = plan
                .max_concurrent_negotiations
                .saturating_sub(actual_active) as usize;
            let mut negotiations_this_club = 0usize;

            for shortlist in &plan.shortlists {
                if negotiations_this_club >= slots_available {
                    break;
                }

                if shortlist.has_pursuing_candidate() {
                    continue;
                }

                if shortlist.all_exhausted() {
                    continue;
                }

                let candidate = match shortlist.current_candidate() {
                    Some(c) if c.status == ShortlistCandidateStatus::Available => c,
                    _ => continue,
                };

                let player_id = candidate.player_id;

                if country
                    .transfer_market
                    .has_active_negotiation_for(player_id, club.id)
                {
                    continue;
                }

                // Skip players on loan contracts — they belong to another club
                // Skip recently signed players — their club has a plan for them
                let (is_on_loan, is_protected) = Self::find_player_in_country(country, player_id)
                    .map(|p| {
                        (
                            p.is_on_loan(),
                            p.is_transfer_protected(date, current_window),
                        )
                    })
                    .unwrap_or((false, false));
                if is_on_loan || is_protected {
                    continue;
                }

                let selling_club_id = country
                    .clubs
                    .iter()
                    .find(|c| c.teams.contains_player(player_id))
                    .map(|c| c.id);

                let selling_club_id = match selling_club_id {
                    Some(id) if id != club.id => id,
                    _ => continue, // Foreign players handled by initiate_foreign_negotiations
                };

                // Rivalry is a deal friction, not an absolute block. A weaker
                // rival approaching a giant has essentially no chance; a club
                // at parity or above can still force the move through by
                // paying a premium or on a reputation-gap flinch. The penalty
                // is applied during resolve_initial_approach via is_rival flag.
                let is_rival = club.is_rival(selling_club_id);

                // ──────────────────────────────────────────────────
                // SMART BUY/LOAN DECISION
                // The DoF decides the approach based on context:
                // - Club reputation tier
                // - Budget vs player value
                // - Transfer request reason
                // - Whether the player is loan-listed
                // - Player age and potential
                // ──────────────────────────────────────────────────

                let request = plan
                    .transfer_requests
                    .iter()
                    .find(|r| r.id == shortlist.transfer_request_id);

                // Scout-side context for this candidate — believed
                // ability/potential from monitoring rows or reports.
                // Drives both the buy/loan decision and (further down)
                // the offer strategy. Hidden PA is never consulted.
                let monitoring = plan
                    .scout_monitoring
                    .iter()
                    .find(|m| m.player_id == player_id);
                let scouting_report = plan
                    .scouting_reports
                    .iter()
                    .find(|r| r.player_id == player_id);
                let scout_assessed = monitoring
                    .map(|m| (m.current_assessed_ability, m.current_assessed_potential))
                    .or_else(|| {
                        scouting_report.map(|r| (r.assessed_ability, r.assessed_potential))
                    });
                let scout_confidence = monitoring
                    .map(|m| m.confidence)
                    .or_else(|| scouting_report.map(|r| r.confidence));

                let target = Self::find_player_in_country(country, player_id);
                let player_age = target.map(|p| p.age(date)).unwrap_or(25);
                // "Gettable" signals: a peer/bigger seller only parts with
                // a prospect who is listed, wants out, or barely plays.
                let target_available = target
                    .map(|p| {
                        p.statuses.has(PlayerStatusType::Lst)
                            || p.statuses.has(PlayerStatusType::Loa)
                            || p.statuses.has(PlayerStatusType::Req)
                            || p.statuses.has(PlayerStatusType::Unh)
                            || (p.statistics.played + p.statistics.played_subs) < 10
                    })
                    .unwrap_or(false);
                let expected_wage = target
                    .map(|p| {
                        WageCalculator::expected_annual_wage(
                            p,
                            player_age,
                            buying_rep_score,
                            buying_league_reputation,
                        )
                    })
                    .unwrap_or(0);
                let selling_rep_score = country
                    .clubs
                    .iter()
                    .find(|c| c.id == selling_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.reputation.overall_score())
                    .unwrap_or(0.3);

                let prospect_ctx = ProspectSigningContext {
                    scout_assessed,
                    scout_confidence,
                    prospect_slots_used: plan
                        .prospect_buys_this_window
                        .saturating_add(plan.prospect_pursuits_active),
                    seller_rep_score: selling_rep_score,
                    buyer_rep_score: buying_rep_score,
                    target_available,
                    wage_headroom,
                    expected_wage,
                };

                let approach = Self::determine_transfer_approach(
                    &rep_level,
                    budget,
                    candidate.estimated_fee,
                    request,
                    player_age,
                    date,
                    club.finance.balance.balance,
                    &club.philosophy,
                    &prospect_ctx,
                );

                let is_loan = !matches!(approach, TransferApproach::PermanentTransfer);
                let has_option_to_buy = matches!(approach, TransferApproach::LoanWithOption);
                let is_prospect_purchase = !is_loan
                    && matches!(
                        request.map(|r| &r.reason),
                        Some(TransferNeedReason::DevelopmentSigning)
                    );

                if let Some(player) = Self::find_player_in_country(country, player_id) {
                    let selling_club = country
                        .clubs
                        .iter()
                        .find(|c| c.id == selling_club_id)
                        .unwrap();

                    let buying_aggressiveness =
                        buying_aggressiveness_from_rep(buying_rep_score, selling_rep_score);

                    let allocated_for_move = shortlist.allocated_budget.min(budget);
                    let strategy = ClubTransferStrategy::from_club_context(
                        club.id,
                        Some(CurrencyValue {
                            amount: allocated_for_move,
                            currency: Currency::Usd,
                        }),
                        avg_ability as u16,
                        vec![player.position()],
                        &club.philosophy,
                        &club.board.vision,
                        buying_aggressiveness,
                    );

                    let asking_price = Self::calculate_asking_price(
                        player,
                        country,
                        selling_club,
                        date,
                        price_level,
                    );

                    let actual_asking = if is_loan {
                        let salary_proxy = player
                            .contract
                            .as_ref()
                            .map(|c| c.salary as f64 * 0.35)
                            .unwrap_or(0.0);
                        let loan_fee_rate = if has_option_to_buy { 0.04 } else { 0.07 };
                        CurrencyValue {
                            amount: FormattingUtils::round_fee(
                                (asking_price.amount * loan_fee_rate).max(salary_proxy),
                            ),
                            currency: asking_price.currency.clone(),
                        }
                    } else {
                        asking_price.clone()
                    };

                    // Dossier built from the scout context hoisted above
                    // — strategy uses assessed potential instead of
                    // hidden PA, and respects dossier risk flags. Both
                    // sources are optional — minimal context falls back
                    // to the previous behaviour.
                    let dossier = if monitoring.is_some() || scouting_report.is_some() {
                        Some(Self::build_board_dossier(
                            plan,
                            player_id,
                            shortlist.transfer_request_id,
                        ))
                    } else {
                        None
                    };
                    // Active rival bidders on this player at the
                    // moment. Read once so we don't walk the
                    // negotiation map twice during offer construction.
                    let competition_count: u32 = country
                        .transfer_market
                        .negotiations
                        .values()
                        .filter(|n| {
                            n.player_id == player_id
                                && n.buying_club_id != club.id
                                && matches!(
                                    n.status,
                                    NegotiationStatus::Pending | NegotiationStatus::Countered
                                )
                        })
                        .count() as u32;
                    let strategy_ctx = TransferStrategyContext {
                        date,
                        request,
                        board_dossier: dossier.as_ref(),
                        approach: approach.clone(),
                        buyer_reputation_score: buying_rep_score,
                        seller_reputation_score: selling_rep_score,
                        league_reputation: buying_league_reputation,
                        available_budget: budget,
                        allocated_budget: allocated_for_move,
                        wage_budget_headroom: None,
                        buying_club_balance: club.finance.balance.balance,
                        is_january: Self::is_january_window(date),
                        price_level,
                        shortlist_rank: shortlist
                            .candidates
                            .iter()
                            .position(|c| c.player_id == player_id)
                            .map(|p| p as u8),
                        competition_count: Some(competition_count.min(u8::MAX as u32) as u8),
                        scout_assessed_ability: monitoring
                            .map(|m| m.current_assessed_ability)
                            .or_else(|| scouting_report.map(|r| r.assessed_ability)),
                        scout_assessed_potential: monitoring
                            .map(|m| m.current_assessed_potential)
                            .or_else(|| scouting_report.map(|r| r.assessed_potential)),
                        scout_confidence: monitoring
                            .map(|m| m.confidence)
                            .or_else(|| scouting_report.map(|r| r.confidence)),
                        seller_is_rival: is_rival,
                    };

                    let mut offer = strategy.calculate_initial_offer_with_context(
                        player,
                        &actual_asking,
                        &strategy_ctx,
                    );

                    // Prospect purchases compensate the development club
                    // with a sell-on share — bigger when the seller is
                    // clearly the smaller selling/development side of the
                    // deal. Skipped when the strategy already pledged one.
                    if is_prospect_purchase
                        && !offer
                            .clauses
                            .iter()
                            .any(|c| matches!(c, TransferClause::SellOnClause(_)))
                    {
                        let pct = if selling_rep_score < buying_rep_score * 0.75 {
                            0.15
                        } else {
                            0.10
                        };
                        offer.clauses.push(TransferClause::SellOnClause(pct));
                    }

                    // Add appearance fee clause for loans from high-reputation sellers
                    if is_loan {
                        let selling_rep_level =
                            Self::get_club_reputation_level(country, selling_club_id);
                        match selling_rep_level {
                            ReputationLevel::Elite => {
                                offer.clauses.push(TransferClause::AppearanceFee(
                                    CurrencyValue {
                                        amount: FormattingUtils::round_fee(
                                            offer.base_fee.amount * 0.30,
                                        ),
                                        currency: Currency::Usd,
                                    },
                                    10,
                                ));
                            }
                            ReputationLevel::Continental => {
                                offer.clauses.push(TransferClause::AppearanceFee(
                                    CurrencyValue {
                                        amount: FormattingUtils::round_fee(
                                            offer.base_fee.amount * 0.20,
                                        ),
                                        currency: Currency::Usd,
                                    },
                                    15,
                                ));
                            }
                            _ => {}
                        }
                    }

                    if has_option_to_buy {
                        let option_price = FormattingUtils::round_fee(asking_price.amount * 0.7);
                        offer
                            .clauses
                            .push(TransferClause::LoanOptionToBuy(CurrencyValue {
                                amount: option_price,
                                currency: Currency::Usd,
                            }));
                    }

                    let offered_annual_wage = WageCalculator::expected_annual_wage(
                        player,
                        player.age(date),
                        buying_rep_score,
                        buying_league_reputation,
                    );

                    // Resolve negotiator staff and build reason
                    let negotiator_staff_id = team.staffs.find_negotiator().map(|s| s.id);

                    let scout_report = plan
                        .scouting_reports
                        .iter()
                        .find(|r| r.player_id == player_id);

                    let reason = Self::build_transfer_reason(request, scout_report);

                    // Final plausibility check immediately before creating
                    // the negotiation action. Rejected candidates are
                    // marked unavailable here and a synthetic listing is
                    // never created downstream — see Pass 2 for the
                    // matching skip.
                    let plausibility_inputs = TransferPlausibilityBuilder::from_clubs(
                        country,
                        club,
                        selling_club,
                        player,
                        candidate.estimated_fee,
                        is_loan,
                        true, // unsolicited at the negotiation-entry point
                        date,
                    );
                    if let TransferPlausibilityVerdict::HardReject(_reason) =
                        TransferPlausibilityEvaluator::evaluate(&plausibility_inputs)
                    {
                        plausibility_rejected.push(PlausibilityReject {
                            club_id: club.id,
                            player_id,
                            shortlist_request_id: shortlist.transfer_request_id,
                        });
                        continue;
                    }

                    actions.push(NegotiationAction {
                        club_id: club.id,
                        player_id,
                        selling_club_id,
                        offer,
                        is_loan,
                        has_option_to_buy,
                        is_prospect_purchase,
                        shortlist_request_id: shortlist.transfer_request_id,
                        negotiator_staff_id,
                        reason,
                        player_name: player.full_name.to_string(),
                        selling_club_name: selling_club.name.clone(),
                        player_sold_from: player.sold_from.clone(),
                        offered_annual_wage,
                        buying_league_reputation,
                        is_rival,
                        seller_asking: actual_asking.clone(),
                    });

                    negotiations_this_club += 1;
                }
            }

            // Loan-out candidates are handled by process_loan_out_listings()
        }

        // Pass 2: Start negotiations
        for action in actions {
            let selling_rep = Self::get_club_reputation(country, action.selling_club_id);
            let buying_rep = Self::get_club_reputation(country, action.club_id);
            let (p_age, p_ambition) =
                Self::get_player_negotiation_data(country, action.player_id, date);

            let has_listing = country
                .transfer_market
                .get_listing_by_player(action.player_id)
                .is_some();

            if !has_listing {
                let listing_type = if action.is_loan {
                    TransferListingType::Loan
                } else {
                    TransferListingType::Transfer
                };

                let selling_team_id = country
                    .clubs
                    .iter()
                    .find(|c| c.id == action.selling_club_id)
                    .and_then(|c| c.teams.teams.first())
                    .map(|t| t.id)
                    .unwrap_or(0);

                // The synthetic listing advertises the SELLER's asking price,
                // not the buyer's budget-capped offer. Pricing it off the
                // offer let a cash-poor club define the seller's valuation and
                // walk away with a first-team player for a fraction of his
                // worth; the seller's own asking keeps the acceptance ratio
                // honest (an unaffordable bid now reads as the lowball it is).
                let asking = SyntheticListingPrice::for_unsolicited(&action.seller_asking);

                // Tag this as synthetic — the parent club did not list
                // the player; the negotiation resolver must not grant
                // the "is_listed" acceptance bonus to bids backed by it.
                let listing = TransferListing::new_with_origin(
                    action.player_id,
                    action.selling_club_id,
                    selling_team_id,
                    asking,
                    date,
                    listing_type,
                    TransferListingOrigin::SyntheticUnsolicited,
                );
                country.transfer_market.add_listing(listing);
            }

            if let Some(neg_id) = country.transfer_market.start_negotiation(
                action.player_id,
                action.club_id,
                action.offer,
                date,
                selling_rep,
                buying_rep,
                p_age,
                p_ambition,
            ) {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.is_loan = action.is_loan;
                    negotiation.has_option_to_buy = action.has_option_to_buy;
                    negotiation.is_unsolicited = !has_listing;
                    negotiation.negotiator_staff_id = action.negotiator_staff_id;
                    negotiation.reason = action.reason.clone();
                    negotiation.player_name = action.player_name.clone();
                    negotiation.selling_club_name = action.selling_club_name.clone();
                    negotiation.player_sold_from = action.player_sold_from.clone();
                    negotiation.offered_salary = Some(action.offered_annual_wage);
                    negotiation.buying_league_reputation = action.buying_league_reputation;
                    if action.is_rival {
                        negotiation.reason = format!("{} (rival)", negotiation.reason.trim());
                    }
                }

                if let Some(club) = country.clubs.iter_mut().find(|c| c.id == action.club_id) {
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
                    "Pipeline: Club {} started negotiation for player {} ({})",
                    action.club_id,
                    action.player_id,
                    if action.is_loan { "loan" } else { "transfer" }
                );
            }
        }

        // Apply plausibility rejects: mark each shortlist candidate as
        // unavailable, advance the shortlist past the dud, and notify
        // the pipeline so monitoring rows / request status update. No
        // synthetic listing is created for these — they never reach
        // start_negotiation.
        for reject in plausibility_rejected {
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
            Self::on_negotiation_resolved(country, reject.club_id, reject.player_id, false);
        }

        Self::process_loan_out_listings(country, date);
    }

    /// Determine whether to buy or loan a player.
    /// This is the "DoF decision" - mirrors real-world logic:
    ///
    /// - Elite clubs: Buy starters, loan promising youngsters with options
    /// - Continental clubs: Buy key targets, loan when budget is tight
    /// - National clubs: Buy affordable targets, loan expensive ones
    /// - Regional/Local: Loan most players, only buy cheap or free agents
    /// - If player is loan-listed by their club: always loan
    /// - Development signings: big/wealthy clubs buy the prospect outright
    ///   (Chelsea / Man City / Benfica model), everyone else loans
    /// - January window and negative balance rules bias toward loans
    ///
    /// `prospect` carries the scouts' read, cap usage, seller standing,
    /// and wage room for the DevelopmentSigning branch — all observable
    /// signals, never the hidden biological PA.
    #[allow(clippy::too_many_arguments)]
    fn determine_transfer_approach(
        rep_level: &ReputationLevel,
        budget: f64,
        estimated_fee: f64,
        request: Option<&TransferRequest>,
        player_age: u8,
        date: NaiveDate,
        buying_club_balance: i64,
        philosophy: &ClubPhilosophy,
        prospect: &ProspectSigningContext,
    ) -> TransferApproach {
        let is_january = Self::is_january_window(date);

        let age = player_age;

        // Philosophy-based overrides
        match philosophy {
            ClubPhilosophy::DevelopAndSell => {
                // Develop-and-sell clubs buy young assets and avoid expensive
                // older purchases. Loans are fallback cover, not the default
                // strategy for prospects.
                if age > 28 {
                    return TransferApproach::Loan;
                }
            }
            ClubPhilosophy::SignToCompete => {
                // Prefer permanent transfers even at lower affordability
                // (handled below in affordability section with relaxed thresholds)
            }
            ClubPhilosophy::LoanFocused => {
                // Always prefer loan unless fee < 50k
                let affordability = if estimated_fee > 0.0 {
                    budget / estimated_fee
                } else {
                    10.0
                };
                if estimated_fee >= 50_000.0 || affordability < 0.8 {
                    return TransferApproach::Loan;
                }
            }
            ClubPhilosophy::Balanced => {
                // No override — use existing logic
            }
        }

        // Reason-driven approaches
        if let Some(req) = request {
            match req.reason {
                TransferNeedReason::DevelopmentSigning => {
                    // Big/wealthy clubs acquire the prospect outright and
                    // develop them via loans; smaller or financially
                    // stressed clubs keep the original borrow behaviour.
                    return if Self::prefers_prospect_purchase(
                        rep_level,
                        philosophy,
                        budget,
                        estimated_fee,
                        buying_club_balance,
                        age,
                        prospect,
                    ) {
                        TransferApproach::PermanentTransfer
                    } else {
                        TransferApproach::Loan
                    };
                }
                TransferNeedReason::LoanToFillSquad
                | TransferNeedReason::InjuryCoverLoan
                | TransferNeedReason::OpportunisticLoanUpgrade
                | TransferNeedReason::SquadPadding => {
                    return TransferApproach::Loan;
                }
                TransferNeedReason::ExperiencedHead | TransferNeedReason::CheapReinforcement => {
                    // Prefer loan, but allow cheap buy if very affordable
                    if estimated_fee > 50_000.0 || buying_club_balance < 0 {
                        return TransferApproach::Loan;
                    }
                }
                _ => {}
            }
        }

        let is_critical = request
            .map(|r| r.priority == TransferNeedPriority::Critical)
            .unwrap_or(false);

        // January + Regional/Local/Amateur → always Loan
        if is_january
            && matches!(
                rep_level,
                ReputationLevel::Regional | ReputationLevel::Local | ReputationLevel::Amateur
            )
        {
            return TransferApproach::Loan;
        }

        // January + National + non-Critical request → Loan
        if is_january && *rep_level == ReputationLevel::National && !is_critical {
            return TransferApproach::Loan;
        }

        // Negative balance + non-Elite → Loan
        if buying_club_balance < 0 && *rep_level != ReputationLevel::Elite {
            return TransferApproach::Loan;
        }

        // Can we even afford to buy?
        let affordability = if estimated_fee > 0.0 {
            budget / estimated_fee
        } else {
            10.0 // Free agent, always affordable
        };

        // SignToCompete: accept higher fees, lower affordability thresholds
        if *philosophy == ClubPhilosophy::SignToCompete {
            return if affordability >= 0.75 || (is_critical && affordability >= 0.55) {
                TransferApproach::PermanentTransfer
            } else {
                TransferApproach::LoanWithOption
            };
        }

        match rep_level {
            ReputationLevel::Elite => {
                if affordability >= 0.3 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::LoanWithOption
                }
            }
            ReputationLevel::Continental => {
                if affordability >= 0.4 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.15 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::National => {
                if affordability >= 0.6 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.25 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            ReputationLevel::Regional => {
                if affordability >= 0.7 {
                    TransferApproach::PermanentTransfer
                } else if affordability >= 0.3 {
                    TransferApproach::LoanWithOption
                } else {
                    TransferApproach::Loan
                }
            }
            _ => {
                if affordability >= 1.5 && estimated_fee < 100_000.0 {
                    TransferApproach::PermanentTransfer
                } else {
                    TransferApproach::Loan
                }
            }
        }
    }

    /// DoF decision for a DevelopmentSigning target: buy the prospect
    /// outright instead of borrowing one. Mirrors the Chelsea / Man City /
    /// Benfica model — wealthy clubs acquire high-upside teenagers
    /// permanently, then develop them through loans. The upside read comes
    /// exclusively from the scouts' believed ability/potential
    /// (`prospect.scout_assessed`), never the hidden biological PA: with
    /// no dossier the club has no basis to commit a fee and falls back to
    /// a loan.
    fn prefers_prospect_purchase(
        rep_level: &ReputationLevel,
        philosophy: &ClubPhilosophy,
        budget: f64,
        estimated_fee: f64,
        buying_club_balance: i64,
        player_age: u8,
        prospect: &ProspectSigningContext,
    ) -> bool {
        // Prospect-ownership profile: develop-and-sell clubs at any size
        // (it's their business model), Balanced clubs from National tier
        // up, and the Elite/Continental end of sign-to-compete clubs.
        // Loan-focused clubs borrow — they never stockpile assets.
        let profile_fits = match philosophy {
            ClubPhilosophy::LoanFocused => false,
            ClubPhilosophy::DevelopAndSell => true,
            ClubPhilosophy::Balanced => matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental | ReputationLevel::National
            ),
            ClubPhilosophy::SignToCompete => matches!(
                rep_level,
                ReputationLevel::Elite | ReputationLevel::Continental
            ),
        };
        if !profile_fits {
            return false;
        }

        // Hoarding control: per-window cap counts completed buys plus
        // pursuits still in flight; a failed bid releases its slot on
        // resolution (see on_negotiation_resolved).
        if prospect.prospect_slots_used >= Self::prospect_buy_cap(rep_level) {
            return false;
        }

        // Financial discipline: no prospect shopping in the red, and the
        // fee must fit the transfer budget with headroom to spare —
        // prospect buys are optional investments, not squad needs.
        if buying_club_balance < 0 || estimated_fee > budget * 0.8 {
            return false;
        }

        // Wage discipline: the board's wage mandate must absorb the new
        // contract too, not just the fee.
        if let Some(headroom) = prospect.wage_headroom {
            if prospect.expected_wage as f64 > headroom {
                return false;
            }
        }

        // Development purchases target teenagers / early-twenties only.
        if player_age > 21 {
            return false;
        }

        // Seller standing: equal-or-bigger clubs don't part with happy,
        // playing prospects — require a gettable signal (listed, wants
        // out, barely plays). Smaller development/selling clubs sell
        // their talents as a matter of course.
        if prospect.seller_rep_score >= prospect.buyer_rep_score * 0.9 && !prospect.target_available
        {
            return false;
        }

        // The scouts must believe in a meaningful ceiling above today's
        // level — a confident visible-estimate gap, not raw PA.
        let confident = prospect
            .scout_confidence
            .map(|c| c >= 0.35)
            .unwrap_or(false);
        match prospect.scout_assessed {
            Some((ability, potential)) if confident => potential as i16 - ability as i16 >= 12,
            _ => false,
        }
    }

    /// Per-window cap on permanent prospect purchases by club tier.
    /// Elite clubs run the widest development programmes but still can't
    /// buy unlimited teenagers.
    fn prospect_buy_cap(rep_level: &ReputationLevel) -> u8 {
        match rep_level {
            ReputationLevel::Elite => 3,
            ReputationLevel::Continental => 2,
            _ => 1,
        }
    }

    pub fn on_negotiation_resolved(
        country: &mut Country,
        buying_club_id: u32,
        player_id: u32,
        accepted: bool,
    ) {
        // Was the just-resolved negotiation a permanent move? Read the
        // loan flag off the (still stored) negotiation before the club
        // borrow; the newest entry for the pair is the one resolving.
        // None when no negotiation ever existed (plausibility rejects).
        let resolved_was_loan = country
            .transfer_market
            .negotiations
            .values()
            .filter(|n| n.player_id == player_id && n.buying_club_id == buying_club_id)
            .max_by_key(|n| n.id)
            .map(|n| n.is_loan);

        // Loan-scan deals carry no shortlist, so the shortlist loop below
        // never marks their triggering request fulfilled — a daily-scanning
        // club then keeps re-firing the same need and stacking more loans.
        // Capture the loaned player's position group up front (before the
        // mut club borrow) so we can close the matching open request once
        // the loan lands. Domestic only: a foreign loanee isn't resolvable
        // from this country, and the in-flight depth cap already gates those.
        let loan_filled_group = if resolved_was_loan == Some(true) && accepted {
            Self::find_player_in_country(country, player_id).map(|p| p.position().position_group())
        } else {
            None
        };
        let mut shortlist_matched = false;

        let mut manager_satisfaction_hit: f32 = 0.0;
        if let Some(club) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            let plan = &mut club.transfer_plan;

            // Monitoring lifecycle: mirror the negotiation outcome
            // onto every active monitoring row for this player. Signed
            // = scouts got their man; Lost = the pursuit collapsed.
            if accepted {
                plan.set_monitoring_status_for_player(player_id, ScoutMonitoringStatus::Signed);
            } else {
                plan.set_monitoring_status_for_player(player_id, ScoutMonitoringStatus::Lost);
            }

            // Prospect-purchase slot accounting: a resolved permanent
            // DevelopmentSigning pursuit releases its in-flight slot;
            // only completed buys keep consuming the window cap, so a
            // failed bid doesn't block later prospect buying.
            let prospect_purchase_resolved = resolved_was_loan == Some(false)
                && plan.shortlists.iter().any(|s| {
                    s.candidates.iter().any(|c| c.player_id == player_id)
                        && plan.transfer_requests.iter().any(|r| {
                            r.id == s.transfer_request_id
                                && r.reason == TransferNeedReason::DevelopmentSigning
                        })
                });
            if prospect_purchase_resolved {
                plan.prospect_pursuits_active = plan.prospect_pursuits_active.saturating_sub(1);
                if accepted {
                    plan.prospect_buys_this_window =
                        plan.prospect_buys_this_window.saturating_add(1);
                }
            }

            for shortlist in &mut plan.shortlists {
                if let Some(candidate) = shortlist
                    .candidates
                    .iter_mut()
                    .find(|c| c.player_id == player_id)
                {
                    if accepted {
                        candidate.status = ShortlistCandidateStatus::Signed;

                        if let Some(req) = plan
                            .transfer_requests
                            .iter_mut()
                            .find(|r| r.id == shortlist.transfer_request_id)
                        {
                            req.status = TransferRequestStatus::Fulfilled;
                            // Signing a Critical target is a real morale lift.
                            manager_satisfaction_hit += match req.priority {
                                TransferNeedPriority::Critical => 3.0,
                                TransferNeedPriority::Important => 1.5,
                                TransferNeedPriority::Optional => 0.5,
                            };
                        }
                    } else {
                        candidate.status = ShortlistCandidateStatus::NegotiationFailed;
                        shortlist.advance_to_next();

                        if shortlist.all_exhausted() {
                            if let Some(req) = plan
                                .transfer_requests
                                .iter_mut()
                                .find(|r| r.id == shortlist.transfer_request_id)
                            {
                                if req.priority == TransferNeedPriority::Critical {
                                    // Critical targets re-open — but the
                                    // repeated failure still stings.
                                    req.status = TransferRequestStatus::Pending;
                                    manager_satisfaction_hit -= 2.0;
                                } else {
                                    req.status = TransferRequestStatus::Abandoned;
                                    // Abandoned target = identified need we
                                    // couldn't address. Hits manager morale.
                                    manager_satisfaction_hit -= match req.priority {
                                        TransferNeedPriority::Critical => 4.0,
                                        TransferNeedPriority::Important => 2.5,
                                        TransferNeedPriority::Optional => 0.75,
                                    };
                                }
                            }
                        } else {
                            if let Some(req) = plan
                                .transfer_requests
                                .iter_mut()
                                .find(|r| r.id == shortlist.transfer_request_id)
                            {
                                req.status = TransferRequestStatus::Shortlisted;
                            }
                        }
                    }

                    shortlist_matched = true;
                    break;
                }
            }

            // No shortlist candidate matched and the resolved deal was a
            // completed loan — i.e. a loan-scan signing. Mark the open
            // request in that position group fulfilled so it stops re-firing
            // and stacking further loans on an already-covered position.
            if !shortlist_matched {
                if let Some(group) = loan_filled_group {
                    for request in plan.transfer_requests.iter_mut() {
                        if request.position.position_group() != group {
                            continue;
                        }
                        if matches!(
                            request.status,
                            TransferRequestStatus::Fulfilled | TransferRequestStatus::Abandoned
                        ) {
                            continue;
                        }
                        request.status = TransferRequestStatus::Fulfilled;
                    }
                }
            }

            plan.active_negotiation_count = plan.active_negotiation_count.saturating_sub(1);

            // Push the aggregated delta into the manager's job_satisfaction
            // so a run of failed bids visibly erodes morale. Scoped inside
            // the same `if let Some(club)` so the borrow is still alive.
            if manager_satisfaction_hit.abs() > 0.01 {
                if let Some(main_team) = club.teams.main_mut() {
                    if let Some(mgr) = main_team
                        .staffs
                        .find_mut_by_position(StaffPosition::Manager)
                    {
                        mgr.job_satisfaction =
                            (mgr.job_satisfaction + manager_satisfaction_hit).clamp(0.0, 100.0);
                    }
                }
            }
        }
    }

    /// After a player moves club (transfer, loan, or free agent), remove all
    /// interest data for that player from every club in the country so that
    /// stale scouting/shortlist entries don't linger.
    pub fn clear_player_interest(country: &mut Country, player_id: u32) {
        for club in &mut country.clubs {
            // Ownership check BEFORE the plan borrow: loan-out candidates
            // only survive at the club that currently rosters the player.
            // The development pathway stages a candidate on the buyer in
            // the same tick this sweep runs — wiping it would kill the
            // same-window development loan.
            let owns_player = club.teams.contains_player(player_id);
            let plan = &mut club.transfer_plan;

            // Scouting assignments: drop observations for this player
            for assignment in &mut plan.scouting_assignments {
                assignment.observations.retain(|o| o.player_id != player_id);
            }

            // Scouting reports
            plan.scouting_reports.retain(|r| r.player_id != player_id);

            // Shortlists: remove the candidate entry
            for shortlist in &mut plan.shortlists {
                shortlist.candidates.retain(|c| c.player_id != player_id);
            }

            // Staff recommendations
            plan.staff_recommendations
                .retain(|r| r.player_id != player_id);

            // Loan-out candidates: a moved player is no longer at this
            // club's disposal to be loaned out — unless this IS the club
            // that now owns him.
            plan.loan_out_candidates
                .retain(|c| c.player_id != player_id || owns_player);

            // Drop active monitoring rows so the player no longer
            // appears as "watched" by clubs that didn't sign them.
            plan.scout_monitoring.retain(|m| m.player_id != player_id);
        }
    }

    /// Global post-success cleanup invoked after a successful transfer,
    /// loan, or free-agent signing. Walks every country and:
    ///   - clears scouting / shortlist / monitoring / known-player rows
    ///     for the moved player (`clear_player_interest`),
    ///   - completes any open listings for the player anywhere,
    ///   - rejects active (Pending / Countered) negotiations for the
    ///     player anywhere,
    ///   - syncs the `Wnt` status so the player is no longer flagged
    ///     "wanted" once no real interest remains.
    ///
    /// Completed transfer history is intentionally left untouched so the
    /// player's career page still shows the move on record.
    ///
    /// `clear_player_interest(country)` is per-country and was already
    /// being called at negotiation acceptance time, but only on the
    /// negotiation's owning country — clubs in other countries that had
    /// scout monitoring or shortlist rows kept their stale interest. This
    /// helper closes that gap by sweeping the whole world after the move
    /// actually completes.
    pub fn cleanup_player_transfer_interest(data: &mut SimulatorData, player_id: u32) {
        Self::cleanup_player_transfer_interest_batch(data, std::slice::from_ref(&player_id));
    }

    /// Release-side variant of the world cleanup: same sweep, but open
    /// listings end `Cancelled` instead of `Completed` — nothing was
    /// sold, the club walked the player, and the player page renders the
    /// listing's terminal status. Used by the free-agent release sweep
    /// and the manual move-on-free editor action.
    pub fn cleanup_player_release_interest(data: &mut SimulatorData, player_id: u32) {
        Self::cleanup_player_release_interest_batch(data, std::slice::from_ref(&player_id));
    }

    /// Batched [`cleanup_player_release_interest`] — one world walk for
    /// every player released this tick.
    pub fn cleanup_player_release_interest_batch(data: &mut SimulatorData, player_ids: &[u32]) {
        Self::cleanup_player_interest_batch_with(
            data,
            player_ids,
            TransferListingStatus::Cancelled,
        );
    }

    /// Batched version of [`cleanup_player_transfer_interest`]: walks
    /// every country once and strips interest for every id in
    /// `player_ids` in a single pass, in parallel across countries.
    ///
    /// Phase C used to call the per-player variant inside a tight
    /// `for signed_id in &ops.domestic_signed_ids` loop for every
    /// country result — which meant every country's shortlists got
    /// re-walked once per signed id per country. The orchestrator now
    /// aggregates all signed ids across the world and calls this once
    /// per tick, collapsing O(countries × signings × countries) into
    /// O(countries) work.
    pub fn cleanup_player_transfer_interest_batch(data: &mut SimulatorData, player_ids: &[u32]) {
        Self::cleanup_player_interest_batch_with(
            data,
            player_ids,
            TransferListingStatus::Completed,
        );
    }

    /// World-wide free-agent market-state bump. Applies every country's
    /// offer / reject / block-reason records (aggregated into `batch` by
    /// `WorldMatchdayResult::collect_free_agent_bumps`) in a SINGLE pass
    /// over `data.free_agents`.
    ///
    /// Replaces the per-country bump inside `apply_deferred_transfer_ops`,
    /// which walked the whole pool once for every country
    /// (`O(countries × pool)`). Global dedup realises the documented
    /// "one bump per player per tick" intent: a pool player pursued by
    /// two countries on the same day is now bumped once, not twice.
    ///
    /// Order within the pass matches the old per-country order — offer
    /// before block — so an offer that lands today clears the
    /// failed-approach streak before any same-day block can regrow it.
    pub fn apply_free_agent_market_bumps_batch(
        data: &mut SimulatorData,
        batch: &FreeAgentBumpBatch,
        current_date: NaiveDate,
    ) {
        if batch.is_empty() {
            return;
        }
        let offered: HashSet<u32> = batch.offered_ids.iter().copied().collect();
        let rejected: HashSet<u32> = batch.rejected_ids.iter().copied().collect();
        // Merge block reasons to the highest-ranked (closest-to-signing)
        // reason per player across every country that recorded one.
        let mut merged: HashMap<u32, FreeAgentBlockReason> = HashMap::new();
        for (player_id, reason) in &batch.block_reasons {
            merged
                .entry(*player_id)
                .and_modify(|existing| {
                    if reason.rank() > existing.rank() {
                        *existing = *reason;
                    }
                })
                .or_insert(*reason);
        }

        for player in data.free_agents.iter_mut() {
            if offered.contains(&player.id) {
                player.on_offer_received(current_date);
            }
            if rejected.contains(&player.id) {
                player.on_offer_rejected();
            }
            if let Some(reason) = merged.get(&player.id) {
                player.on_market_blocked(current_date, *reason);
            }
        }
    }

    /// Shared world walk behind the transfer- and release-flavoured
    /// cleanups. `listing_terminal` is the status open listings end in:
    /// `Completed` when the player was signed, `Cancelled` when he was
    /// released with no deal.
    fn cleanup_player_interest_batch_with(
        data: &mut SimulatorData,
        player_ids: &[u32],
        listing_terminal: TransferListingStatus,
    ) {
        if player_ids.is_empty() {
            return;
        }
        // Membership-only set on a whole-world sweep — Fx hashing, the
        // SipHash default was a measurable share of the sweep's CPU.
        let signed: FxHashSet<u32> = player_ids.iter().copied().collect();

        data.continents
            .par_iter_mut()
            .flat_map(|c| c.countries.par_iter_mut())
            .for_each(|country| {
                // Per-club sweep, parallel WITHIN the country too: every
                // retain below touches only its own club's plan/rosters,
                // and with one country task per country the biggest
                // country's serial club walk was the drain phase's
                // pacing straggler. `with_min_len` keeps small countries
                // from shattering into per-club micro-tasks — the sweep
                // per club is tiny and the fan-out churn would otherwise
                // outweigh it.
                country
                    .clubs
                    .par_iter_mut()
                    .with_min_len(8)
                    .for_each(|club| {
                        // Signed players this club now rosters keep their
                        // loan-out candidates — the development pathway
                        // stages them on the buyer in the same tick this
                        // batch sweep runs. Every other club's stale
                        // candidates are still dropped. One roster walk with
                        // set probes — the signed-set side of the check used
                        // to re-walk the roster once per signed id.
                        let owned_signed: Vec<u32> = club
                            .teams
                            .teams
                            .iter()
                            .flat_map(|t| t.players.players.iter())
                            .map(|p| p.id)
                            .filter(|id| signed.contains(id))
                            .collect();
                        let plan = &mut club.transfer_plan;
                        for assignment in &mut plan.scouting_assignments {
                            assignment
                                .observations
                                .retain(|o| !signed.contains(&o.player_id));
                        }
                        plan.scouting_reports
                            .retain(|r| !signed.contains(&r.player_id));
                        for shortlist in &mut plan.shortlists {
                            shortlist
                                .candidates
                                .retain(|c| !signed.contains(&c.player_id));
                        }
                        plan.staff_recommendations
                            .retain(|r| !signed.contains(&r.player_id));
                        plan.loan_out_candidates.retain(|c| {
                            !signed.contains(&c.player_id) || owned_signed.contains(&c.player_id)
                        });
                        plan.scout_monitoring
                            .retain(|m| !signed.contains(&m.player_id));

                        // Team-level selling lists mirror market listings
                        // (stalemate fallback, AI transfer-list manager). A
                        // player who moved or was released must drop off them
                        // too, or the team-transfers page keeps rendering a
                        // stale asking-price row for him.
                        for team in &mut club.teams.teams {
                            team.transfer_list.remove_all(&signed);
                        }

                        // Targeted Wnt reconciliation, folded into the same
                        // club walk. The retains above only removed rows for
                        // `signed` ids, so only THEIR tracked status can have
                        // changed — and by construction none of them is still
                        // tracked at any club. Stripping their Wnt directly
                        // replaces the full per-country `sync_wanted_status`
                        // rebuild that used to run here (the drain phase's
                        // biggest straggler); every other kind of Wnt drift
                        // (window resets, cleared interest) keeps being
                        // reconciled by the daily per-country
                        // `sync_wanted_status` call in the pipeline.
                        for team in &mut club.teams.teams {
                            for player in team.players.players.iter_mut() {
                                if signed.contains(&player.id)
                                    && player.statuses.has(PlayerStatusType::Wnt)
                                {
                                    player.statuses.remove(PlayerStatusType::Wnt);
                                }
                            }
                        }
                    });

                for listing in country.transfer_market.listings.iter_mut() {
                    if signed.contains(&listing.player_id)
                        && listing.status != TransferListingStatus::Completed
                        && listing.status != TransferListingStatus::Cancelled
                    {
                        listing.status = listing_terminal.clone();
                    }
                }

                for negotiation in country.transfer_market.negotiations.values_mut() {
                    if signed.contains(&negotiation.player_id)
                        && (negotiation.status == NegotiationStatus::Pending
                            || negotiation.status == NegotiationStatus::Countered)
                    {
                        negotiation.status = NegotiationStatus::Rejected;
                    }
                }
            });
    }

    /// Reconcile `Wnt` statuses with actual interest. `Wnt` is added during
    /// scouting but has no intrinsic expiry — when window resets wipe all
    /// interest tracking, the status lingers and players appear "Wanted"
    /// with no interested clubs behind it. This walks the country once per
    /// invocation, collects the set of still-tracked player ids, and strips
    /// `Wnt` from anyone who is no longer on any club's radar.
    pub fn sync_wanted_status(country: &mut Country) {
        // Membership-only set rebuilt from every plan row in the country —
        // the SipHash inserts were the dominant cost of this walk.
        let mut tracked: FxHashSet<u32> = FxHashSet::default();
        for club in &country.clubs {
            let plan = &club.transfer_plan;
            for assignment in &plan.scouting_assignments {
                for obs in &assignment.observations {
                    tracked.insert(obs.player_id);
                }
            }
            for r in &plan.scouting_reports {
                tracked.insert(r.player_id);
            }
            for s in &plan.shortlists {
                for c in &s.candidates {
                    tracked.insert(c.player_id);
                }
            }
            for r in &plan.staff_recommendations {
                tracked.insert(r.player_id);
            }
            // Active monitoring rows count as live interest — the
            // recruitment department is still watching the player even
            // if no scouting assignment row exists yet.
            for m in &plan.scout_monitoring {
                if m.is_active_interest() {
                    tracked.insert(m.player_id);
                }
            }
        }

        for club in &mut country.clubs {
            for team in &mut club.teams.teams {
                for player in team.players.players.iter_mut() {
                    if player.statuses.has(PlayerStatusType::Wnt) && !tracked.contains(&player.id) {
                        player.statuses.remove(PlayerStatusType::Wnt);
                    }
                }
            }
        }
    }

    /// Resolve which foreign club currently holds `player_id`, for a
    /// buyer based in `buyer_country_id`. Tries the O(1) global
    /// player-location index first and verifies the hit (the index can
    /// be one tick stale after an intra-tick move), falling back to a
    /// full world scan only on a miss or a stale entry. Returns
    /// `(country_id, club_id, price_level, continent_id, country_code)`
    /// for the selling side, or `None` when the player can't be located
    /// in any country other than the buyer's.
    ///
    /// Replaces the previous per-candidate triple-nested world scan
    /// (`O(candidates × all_clubs)`); the index hit is the common path.
    fn resolve_foreign_player_club(
        data: &SimulatorData,
        buyer_country_id: u32,
        player_id: u32,
    ) -> Option<(u32, u32, f32, u32, String)> {
        // Fast path: global player-location index (O(1)).
        if let Some((_continent_id, loc_country_id, loc_club_id, _team_id)) = data
            .indexes
            .as_ref()
            .and_then(|idx| idx.get_player_location(player_id))
        {
            if loc_country_id != buyer_country_id {
                if let Some(country) = data.country(loc_country_id) {
                    // Verify the player really is at this club — the
                    // index may be one tick stale after an intra-tick
                    // move; on a stale hit fall through to the scan.
                    let present = country
                        .club(loc_club_id)
                        .map(|c| c.teams.contains_player(player_id))
                        .unwrap_or(false);
                    if present {
                        return Some((
                            country.id,
                            loc_club_id,
                            country.settings.pricing.price_level,
                            country.continent_id,
                            country.code.clone(),
                        ));
                    }
                }
            }
            // Index hit pointed at the buyer's own country or was stale —
            // fall through to the authoritative scan below.
        }

        // Slow path: full world scan, foreign-only (skips the buyer's
        // country). Reached only on an index miss or a stale entry.
        for continent in &data.continents {
            for country in &continent.countries {
                if country.id == buyer_country_id {
                    continue;
                }
                for club in &country.clubs {
                    if club.teams.contains_player(player_id) {
                        return Some((
                            country.id,
                            club.id,
                            country.settings.pricing.price_level,
                            country.continent_id,
                            country.code.clone(),
                        ));
                    }
                }
            }
        }
        None
    }

    pub fn initiate_foreign_negotiations(
        data: &mut SimulatorData,
        country_id: u32,
        date: NaiveDate,
    ) {
        // Pass 1: Read — collect foreign candidates from shortlists
        struct ForeignCandidate {
            buying_club_id: u32,
            player_id: u32,
            shortlist_request_id: u32,
        }

        let mut candidates: Vec<ForeignCandidate> = Vec::new();

        if let Some(country) = data.country(country_id) {
            for club in &country.clubs {
                let plan = &club.transfer_plan;
                if !plan.initialized || !plan.can_start_negotiation() {
                    continue;
                }

                let actual_active = country
                    .transfer_market
                    .active_negotiation_count_for_club(club.id);
                if actual_active >= plan.max_concurrent_negotiations {
                    continue;
                }

                for shortlist in &plan.shortlists {
                    if shortlist.has_pursuing_candidate() || shortlist.all_exhausted() {
                        continue;
                    }

                    let candidate = match shortlist.current_candidate() {
                        Some(c) if c.status == ShortlistCandidateStatus::Available => c,
                        _ => continue,
                    };

                    // Only process if player is NOT in the local country
                    let is_local =
                        Self::find_player_in_country(country, candidate.player_id).is_some();
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
                }
            }
        }

        if candidates.is_empty() {
            return;
        }

        // Pass 2: Resolve — find each player globally, compute offers
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
            reason: String,
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
        }

        let mut resolved: Vec<ResolvedNeg> = Vec::new();
        // Foreign candidates the final cross-border gate refuses — marked
        // unavailable after the write pass so the shortlist advances instead
        // of re-picking an impossible target (mirrors the domestic gate).
        let mut foreign_rejected: Vec<PlausibilityReject> = Vec::new();

        for cand in candidates {
            // Resolve the player's current foreign club via the O(1)
            // global index (verified, with a full-scan fallback for a
            // stale entry) instead of re-walking the whole world per
            // candidate.
            let found = Self::resolve_foreign_player_club(data, country_id, cand.player_id);

            let (
                sell_country_id,
                sell_club_id,
                sell_price_level,
                sell_continent_id,
                sell_country_code,
            ) = match found {
                Some(v) => v,
                None => continue,
            };

            let sell_country = match data.country(sell_country_id) {
                Some(c) => c,
                None => continue,
            };
            let player = match Self::find_player_in_country(sell_country, cand.player_id) {
                Some(p) => p,
                None => continue,
            };
            if player.is_on_loan() {
                continue;
            }
            // Use the selling-side country reference (already in scope as
            // `sell_country`) so its country-specific calendar is honoured
            // — the buyer-side window doesn't apply when the player sits
            // in a different country's market.
            let sell_window = TransferWindowManager::for_country(sell_country, date)
                .current_window_dates(sell_country_id, date);
            if player.is_transfer_protected(date, sell_window) {
                continue;
            }

            let sell_club = match sell_country.clubs.iter().find(|c| c.id == sell_club_id) {
                Some(c) => c,
                None => continue,
            };
            let asking_price = Self::calculate_asking_price(
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

            let buy_country = match data.country(country_id) {
                Some(c) => c,
                None => continue,
            };
            let buy_club = match buy_country
                .clubs
                .iter()
                .find(|c| c.id == cand.buying_club_id)
            {
                Some(c) => c,
                None => continue,
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

            let approach = Self::determine_transfer_approach(
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
            );
            let assessment = TransferMovePlausibility::assess(&plausibility_inputs);
            if !assessment.reaches(TransferMoveStage::CanStartNegotiation) {
                debug!(
                    "Foreign negotiation suppressed: club {} won't pursue {} ({}) from {} — {}",
                    cand.buying_club_id,
                    cand.player_id,
                    player_name,
                    selling_club_name,
                    assessment.diagnostics.explain()
                );
                foreign_rejected.push(PlausibilityReject {
                    club_id: cand.buying_club_id,
                    player_id: cand.player_id,
                    shortlist_request_id: cand.shortlist_request_id,
                });
                continue;
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

            let actual_asking = if is_loan {
                let salary_proxy = player
                    .contract
                    .as_ref()
                    .map(|c| c.salary as f64 * 0.35)
                    .unwrap_or(0.0);
                let loan_fee_rate = if has_option_to_buy { 0.04 } else { 0.07 };
                CurrencyValue {
                    amount: FormattingUtils::round_fee(
                        (asking_price.amount * loan_fee_rate).max(salary_proxy),
                    ),
                    currency: asking_price.currency.clone(),
                }
            } else {
                asking_price.clone()
            };

            let avg_ability: u8 = buy_club
                .teams
                .teams
                .first()
                .map(|t| {
                    let avg = t.players.current_ability_avg();
                    if avg == 0 { 50 } else { avg }
                })
                .unwrap_or(50);

            let strategy = ClubTransferStrategy::from_club_context(
                cand.buying_club_id,
                Some(CurrencyValue {
                    amount: budget,
                    currency: Currency::Usd,
                }),
                avg_ability as u16,
                vec![player.position()],
                &buy_club.philosophy,
                &buy_club.board.vision,
                buying_aggressiveness_from_rep(buying_rep, selling_rep),
            );

            // Dossier built from the scout context hoisted above — the
            // dossier helper resolves the rest from the same plan.
            let dossier = if monitoring.is_some() || scouting_report.is_some() {
                Some(Self::build_board_dossier(
                    &buy_club.transfer_plan,
                    cand.player_id,
                    cand.shortlist_request_id,
                ))
            } else {
                None
            };
            let strategy_ctx = TransferStrategyContext {
                date,
                request,
                board_dossier: dossier.as_ref(),
                approach: approach.clone(),
                buyer_reputation_score: buying_rep,
                seller_reputation_score: selling_rep,
                league_reputation: buying_league_reputation,
                available_budget: budget,
                allocated_budget: budget,
                wage_budget_headroom: None,
                buying_club_balance: buy_club.finance.balance.balance,
                is_january: Self::is_january_window(date),
                price_level: sell_price_level,
                shortlist_rank: None,
                competition_count: None,
                scout_assessed_ability: monitoring
                    .map(|m| m.current_assessed_ability)
                    .or_else(|| scouting_report.map(|r| r.assessed_ability)),
                scout_assessed_potential: monitoring
                    .map(|m| m.current_assessed_potential)
                    .or_else(|| scouting_report.map(|r| r.assessed_potential)),
                scout_confidence: monitoring
                    .map(|m| m.confidence)
                    .or_else(|| scouting_report.map(|r| r.confidence)),
                seller_is_rival: false,
            };

            let mut offer = strategy.calculate_initial_offer_with_context(
                player,
                &actual_asking,
                &strategy_ctx,
            );

            // Foreign prospect purchases carry the same sell-on
            // compensation as domestic ones — see initiate_negotiations.
            if is_prospect_purchase
                && !offer
                    .clauses
                    .iter()
                    .any(|c| matches!(c, TransferClause::SellOnClause(_)))
            {
                let pct = if selling_rep < buying_rep * 0.75 {
                    0.15
                } else {
                    0.10
                };
                offer.clauses.push(TransferClause::SellOnClause(pct));
            }

            if has_option_to_buy {
                let option_price = FormattingUtils::round_fee(asking_price.amount * 0.7);
                offer
                    .clauses
                    .push(TransferClause::LoanOptionToBuy(CurrencyValue {
                        amount: option_price,
                        currency: Currency::Usd,
                    }));
            }

            let offered_annual_wage = WageCalculator::expected_annual_wage(
                player,
                player_age,
                buying_rep,
                buying_league_reputation,
            );

            let reason = if is_loan {
                "Loan signing".to_string()
            } else {
                "Transfer signing".to_string()
            };

            resolved.push(ResolvedNeg {
                buying_club_id: cand.buying_club_id,
                selling_country_id: sell_country_id,
                selling_continent_id: sell_continent_id,
                selling_country_code: sell_country_code,
                selling_club_id: sell_club_id,
                player_id: cand.player_id,
                is_loan,
                has_option_to_buy,
                is_prospect_purchase,
                offer,
                reason,
                shortlist_request_id: cand.shortlist_request_id,
                selling_rep,
                buying_rep,
                player_age,
                player_ambition,
                asking_price,
                player_name,
                selling_club_name,
                player_sold_from: player.sold_from.clone(),
                offered_annual_wage,
                buying_league_reputation,
                foreign_terms_floor_blocked,
                foreign_seller_importance,
            });
        }

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
                    negotiation.reason = action.reason;
                    negotiation.selling_country_id = Some(action.selling_country_id);
                    negotiation.selling_continent_id = Some(action.selling_continent_id);
                    negotiation.selling_country_code = action.selling_country_code;
                    negotiation.player_sold_from = action.player_sold_from;
                    negotiation.player_name = action.player_name;
                    negotiation.selling_club_name = action.selling_club_name;
                    negotiation.offered_salary = Some(action.offered_annual_wage);
                    negotiation.buying_league_reputation = action.buying_league_reputation;
                    negotiation.foreign_terms_floor_blocked = action.foreign_terms_floor_blocked;
                    negotiation.foreign_seller_importance = Some(action.foreign_seller_importance);
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
                    Self::on_negotiation_resolved(country, reject.club_id, reject.player_id, false);
                }
            }
        }
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::market::{TransferListing, TransferListingStatus, TransferListingType};
    use crate::transfers::negotiation::{NegotiationStatus, TransferNegotiation};
    use crate::transfers::offer::TransferOffer;
    use crate::transfers::pipeline::recruitment::{
        ScoutMonitoringSource, ScoutMonitoringStatus, ScoutPlayerMonitoring,
    };
    use crate::transfers::pipeline::{
        ShortlistCandidate, ShortlistCandidateStatus, TransferShortlist,
    };
    use crate::transfers::{CompletedTransfer, TransferType};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PlayerPositionType,
        TeamCollection,
    };
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn make_club(id: u32, name: &str) -> Club {
        Club::new(
            id,
            name.to_string(),
            Location::new(1),
            ClubFinances::new(1_000_000, Vec::new()),
            ClubAcademy::new(3),
            ClubStatus::Professional,
            ClubColors::default(),
            TeamCollection::new(Vec::new()),
            ClubFacilities::default(),
        )
    }

    fn make_league(id: u32, slug: &str) -> League {
        League::new(
            id,
            "L".to_string(),
            slug.to_string(),
            1,
            500,
            LeagueSettings {
                season_starting_half: DayMonthPeriod::new(1, 8, 31, 12),
                season_ending_half: DayMonthPeriod::new(1, 1, 31, 5),
                tier: 1,
                promotion_spots: 0,
                relegation_spots: 0,
                league_group: None,
            },
            false,
        )
    }

    fn make_country(id: u32, code: &str, slug: &str, clubs: Vec<Club>) -> Country {
        Country::builder()
            .id(id)
            .code(code.to_string())
            .slug(slug.to_string())
            .name(slug.to_string())
            .continent_id(1)
            .leagues(LeagueCollection::new(vec![make_league(id, slug)]))
            .clubs(clubs)
            .build()
            .unwrap()
    }

    fn make_simulator(date: NaiveDate, countries: Vec<Country>) -> SimulatorData {
        let continent = Continent::new(1, "Europe".to_string(), countries, Vec::new());
        SimulatorData::new(
            date.and_hms_opt(12, 0, 0).unwrap(),
            vec![continent],
            GlobalCompetitions::new(Vec::new()),
        )
    }

    fn put_monitoring(
        club: &mut Club,
        scout_id: u32,
        player_id: u32,
        status: ScoutMonitoringStatus,
    ) {
        let id = club.transfer_plan.next_monitoring_id();
        let mut row = ScoutPlayerMonitoring::new(
            id,
            scout_id,
            player_id,
            ScoutMonitoringSource::TransferRequest,
            d(2026, 6, 1),
        );
        row.status = status;
        club.transfer_plan.scout_monitoring.push(row);
    }

    fn put_shortlist_candidate(club: &mut Club, request_id: u32, player_id: u32) {
        let mut sl = TransferShortlist::new(request_id, 0.0);
        sl.candidates.push(ShortlistCandidate {
            player_id,
            score: 0.5,
            estimated_fee: 0.0,
            status: ShortlistCandidateStatus::Available,
        });
        club.transfer_plan.shortlists.push(sl);
    }

    fn put_listing(
        country: &mut Country,
        player_id: u32,
        club_id: u32,
        status: TransferListingStatus,
    ) {
        let mut listing = TransferListing::new(
            player_id,
            club_id,
            0,
            CurrencyValue::new(1_000_000.0, Currency::Usd),
            d(2026, 6, 1),
            TransferListingType::Transfer,
        );
        listing.status = status;
        country.transfer_market.listings.push(listing);
    }

    fn put_negotiation(
        country: &mut Country,
        neg_id: u32,
        player_id: u32,
        buying_club_id: u32,
        selling_club_id: u32,
        status: NegotiationStatus,
    ) {
        let offer = TransferOffer::new(
            CurrencyValue::new(1_000_000.0, Currency::Usd),
            buying_club_id,
            d(2026, 6, 1),
        );
        let mut neg = TransferNegotiation::new(
            neg_id,
            player_id,
            0,
            selling_club_id,
            buying_club_id,
            offer,
            d(2026, 6, 1),
            500.0,
            500.0,
            25,
            10.0,
        );
        neg.status = status;
        country.transfer_market.negotiations.insert(neg_id, neg);
    }

    fn put_history(country: &mut Country, player_id: u32, from_club_id: u32, to_club_id: u32) {
        country
            .transfer_market
            .transfer_history
            .push(CompletedTransfer::new(
                player_id,
                "Player".to_string(),
                from_club_id,
                0,
                "From".to_string(),
                to_club_id,
                "To".to_string(),
                d(2026, 6, 5),
                CurrencyValue::new(2_000_000.0, Currency::Usd),
                TransferType::Permanent,
            ));
    }

    #[test]
    fn cleanup_clears_active_monitoring_and_shortlist() {
        // Buying club has a Negotiating-state monitoring row plus a
        // shortlist entry for the player. After the centralized cleanup
        // both should be gone — the UI should not surface the player as
        // actively monitored.
        let player_id: u32 = 100;
        let buyer_club_id: u32 = 1;
        let other_club_id: u32 = 2;
        let mut buyer = make_club(buyer_club_id, "Buyer");
        put_monitoring(
            &mut buyer,
            11,
            player_id,
            ScoutMonitoringStatus::Negotiating,
        );
        put_shortlist_candidate(&mut buyer, 1, player_id);

        // A second domestic club had the player on its scouting radar.
        let mut other = make_club(other_club_id, "Other");
        put_monitoring(&mut other, 12, player_id, ScoutMonitoringStatus::Active);

        let country = make_country(1, "UR", "uruguay", vec![buyer, other]);
        let mut data = make_simulator(d(2026, 6, 5), vec![country]);

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, player_id);

        let country = data.country(1).unwrap();
        for club in &country.clubs {
            assert!(
                club.transfer_plan
                    .scout_monitoring
                    .iter()
                    .all(|m| m.player_id != player_id),
                "club {} still has stale monitoring rows for player {}",
                club.id,
                player_id
            );
            for sl in &club.transfer_plan.shortlists {
                assert!(
                    sl.candidates.iter().all(|c| c.player_id != player_id),
                    "club {} still has shortlist candidate for player {}",
                    club.id,
                    player_id
                );
            }
        }
    }

    #[test]
    fn cleanup_completes_listings_and_rejects_active_negotiations() {
        let player_id: u32 = 200;
        let selling_club_id: u32 = 3;
        let buying_club_id: u32 = 4;
        let losing_club_id: u32 = 5;

        let buyer = make_club(buying_club_id, "Buyer");
        let seller = make_club(selling_club_id, "Seller");
        let losing_bidder = make_club(losing_club_id, "Loser");
        let mut country = make_country(1, "EN", "england", vec![buyer, seller, losing_bidder]);

        // An open listing — must be marked Completed.
        put_listing(
            &mut country,
            player_id,
            selling_club_id,
            TransferListingStatus::Available,
        );
        // The buyer's negotiation got accepted — leave Accepted alone but
        // a parallel bid from the losing club is still Pending and must
        // be rejected.
        put_negotiation(
            &mut country,
            10,
            player_id,
            buying_club_id,
            selling_club_id,
            NegotiationStatus::Accepted,
        );
        put_negotiation(
            &mut country,
            11,
            player_id,
            losing_club_id,
            selling_club_id,
            NegotiationStatus::Pending,
        );
        put_history(&mut country, player_id, selling_club_id, buying_club_id);

        let mut data = make_simulator(d(2026, 6, 5), vec![country]);

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, player_id);

        let country = data.country(1).unwrap();
        // All listings for the player are Completed.
        for listing in &country.transfer_market.listings {
            if listing.player_id == player_id {
                assert_eq!(
                    listing.status,
                    TransferListingStatus::Completed,
                    "listing for player {} not completed",
                    player_id
                );
            }
        }
        // Pending negotiation is now Rejected; Accepted stays Accepted.
        let losing = &country.transfer_market.negotiations[&11];
        assert_eq!(losing.status, NegotiationStatus::Rejected);
        let winning = &country.transfer_market.negotiations[&10];
        assert_eq!(winning.status, NegotiationStatus::Accepted);
        // Transfer history must NOT be deleted.
        assert!(
            country
                .transfer_market
                .transfer_history
                .iter()
                .any(|t| t.player_id == player_id),
            "completed transfer history for player {} was deleted",
            player_id
        );
    }

    #[test]
    fn cross_country_cleanup_clears_both_sides() {
        let player_id: u32 = 300;
        let selling_club_id: u32 = 6;
        let buying_club_id: u32 = 7;
        let third_party_club_id: u32 = 8;

        let mut seller = make_club(selling_club_id, "Seller");
        // Seller's home country had an open listing for the player.
        let mut buyer = make_club(buying_club_id, "Buyer");
        put_monitoring(
            &mut buyer,
            21,
            player_id,
            ScoutMonitoringStatus::Negotiating,
        );
        put_shortlist_candidate(&mut buyer, 1, player_id);

        // A club in a third country scouted the same player — its
        // monitoring row must be cleared too.
        let mut third_party = make_club(third_party_club_id, "ThirdParty");
        put_monitoring(
            &mut third_party,
            31,
            player_id,
            ScoutMonitoringStatus::Active,
        );
        // Selling-side staff also had an internal monitoring (e.g. their
        // own academy DoF flagged the player on the way out).
        put_monitoring(&mut seller, 41, player_id, ScoutMonitoringStatus::Active);

        let mut selling_country = make_country(1, "AR", "argentina", vec![seller]);
        // Selling country listing pre-completion.
        put_listing(
            &mut selling_country,
            player_id,
            selling_club_id,
            TransferListingStatus::InNegotiation,
        );
        let buying_country = make_country(2, "ES", "spain", vec![buyer]);
        let third_country = make_country(3, "PT", "portugal", vec![third_party]);

        let mut data = make_simulator(
            d(2026, 6, 5),
            vec![selling_country, buying_country, third_country],
        );

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, player_id);

        // Verify each country has been swept clean.
        for cont in &data.continents {
            for country in &cont.countries {
                for club in &country.clubs {
                    assert!(
                        club.transfer_plan
                            .scout_monitoring
                            .iter()
                            .all(|m| m.player_id != player_id),
                        "country {} club {} still has monitoring for player {}",
                        country.id,
                        club.id,
                        player_id
                    );
                    for sl in &club.transfer_plan.shortlists {
                        assert!(
                            sl.candidates.iter().all(|c| c.player_id != player_id),
                            "country {} club {} still has shortlist candidate",
                            country.id,
                            club.id
                        );
                    }
                }
                for listing in &country.transfer_market.listings {
                    if listing.player_id == player_id {
                        assert_eq!(
                            listing.status,
                            TransferListingStatus::Completed,
                            "country {} listing for player {} not completed",
                            country.id,
                            player_id
                        );
                    }
                }
            }
        }
    }

    /// Stage a DevelopmentSigning request + shortlist + in-flight
    /// negotiation so the slot-accounting tests can resolve it.
    fn put_prospect_pursuit(
        country: &mut Country,
        club_idx: usize,
        request_id: u32,
        player_id: u32,
        neg_id: u32,
    ) {
        let club_id = country.clubs[club_idx].id;
        put_shortlist_candidate(&mut country.clubs[club_idx], request_id, player_id);
        country.clubs[club_idx]
            .transfer_plan
            .transfer_requests
            .push(TransferRequest::new(
                request_id,
                PlayerPositionType::Striker,
                TransferNeedPriority::Optional,
                TransferNeedReason::DevelopmentSigning,
                40,
                70,
                2_000_000.0,
            ));
        country.clubs[club_idx]
            .transfer_plan
            .prospect_pursuits_active = 1;
        put_negotiation(
            country,
            neg_id,
            player_id,
            club_id,
            99, // arbitrary selling club id
            NegotiationStatus::Rejected,
        );
    }

    /// A failed prospect-purchase bid must release the in-flight slot
    /// without consuming the window cap — otherwise one collapsed
    /// negotiation permanently blocks later prospect buying.
    #[test]
    fn failed_prospect_purchase_releases_window_slot() {
        let player_id: u32 = 600;
        let buyer = make_club(1, "Buyer");
        let mut country = make_country(1, "EN", "england", vec![buyer]);
        put_prospect_pursuit(&mut country, 0, 7, player_id, 10);

        PipelineProcessor::on_negotiation_resolved(&mut country, 1, player_id, false);

        let plan = &country.clubs[0].transfer_plan;
        assert_eq!(
            plan.prospect_pursuits_active, 0,
            "failed bid must release the pursuit slot"
        );
        assert_eq!(
            plan.prospect_buys_this_window, 0,
            "failed bid is not a completed buy"
        );
    }

    /// An accepted prospect purchase converts the in-flight slot into a
    /// completed buy that keeps consuming the window cap.
    #[test]
    fn accepted_prospect_purchase_converts_slot_into_completed_buy() {
        let player_id: u32 = 601;
        let buyer = make_club(1, "Buyer");
        let mut country = make_country(1, "EN", "england", vec![buyer]);
        put_prospect_pursuit(&mut country, 0, 7, player_id, 11);

        PipelineProcessor::on_negotiation_resolved(&mut country, 1, player_id, true);

        let plan = &country.clubs[0].transfer_plan;
        assert_eq!(plan.prospect_pursuits_active, 0);
        assert_eq!(
            plan.prospect_buys_this_window, 1,
            "completed buy must keep consuming the window cap"
        );
    }

    #[test]
    fn cleanup_preserves_unrelated_player_interest() {
        // Two players: 400 has been signed; 500 is unrelated. The sweep
        // for 400 must NOT touch 500's monitoring / shortlist entries.
        let signed_id: u32 = 400;
        let other_id: u32 = 500;

        let mut buyer = make_club(1, "Buyer");
        put_monitoring(
            &mut buyer,
            11,
            signed_id,
            ScoutMonitoringStatus::Negotiating,
        );
        put_monitoring(&mut buyer, 12, other_id, ScoutMonitoringStatus::Active);
        put_shortlist_candidate(&mut buyer, 1, signed_id);
        put_shortlist_candidate(&mut buyer, 2, other_id);

        let country = make_country(1, "FR", "france", vec![buyer]);
        let mut data = make_simulator(d(2026, 6, 5), vec![country]);

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, signed_id);

        let buyer = &data.country(1).unwrap().clubs[0];
        // Signed player interest is gone.
        assert!(
            buyer
                .transfer_plan
                .scout_monitoring
                .iter()
                .all(|m| m.player_id != signed_id)
        );
        // Other player interest survives.
        assert!(
            buyer
                .transfer_plan
                .scout_monitoring
                .iter()
                .any(|m| m.player_id == other_id),
            "monitoring for unrelated player wiped"
        );
        let still_shortlisted = buyer
            .transfer_plan
            .shortlists
            .iter()
            .any(|s| s.candidates.iter().any(|c| c.player_id == other_id));
        assert!(still_shortlisted, "unrelated player removed from shortlist");
    }
}

#[cfg(test)]
mod prospect_approach_tests {
    use super::*;
    use crate::PlayerPositionType;
    use crate::transfers::pipeline::TransferApproach;
    use chrono::NaiveDate;

    /// Fixtures for the prospect buy-vs-loan decision matrix. Grouped on
    /// a unit struct per the project's no-free-helpers convention.
    struct ApproachFixtures;

    impl ApproachFixtures {
        fn summer() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()
        }

        fn development_request() -> TransferRequest {
            TransferRequest::new(
                1,
                PlayerPositionType::Striker,
                TransferNeedPriority::Optional,
                TransferNeedReason::DevelopmentSigning,
                40,
                70,
                2_000_000.0,
            )
        }

        fn loan_fill_request() -> TransferRequest {
            TransferRequest::new(
                2,
                PlayerPositionType::Striker,
                TransferNeedPriority::Important,
                TransferNeedReason::LoanToFillSquad,
                40,
                70,
                0.0,
            )
        }

        /// Prospect context with healthy defaults: a confident dossier,
        /// no slots used, a small development-club seller, no wage
        /// mandate. Negative tests perturb individual fields.
        fn ctx(
            rep: &ReputationLevel,
            scout: Option<(u8, u8)>,
            slots_used: u8,
        ) -> ProspectSigningContext {
            let buyer_rep_score = match rep {
                ReputationLevel::Elite => 0.90,
                ReputationLevel::Continental => 0.72,
                ReputationLevel::National => 0.57,
                ReputationLevel::Regional => 0.40,
                _ => 0.20,
            };
            ProspectSigningContext {
                scout_assessed: scout,
                scout_confidence: scout.map(|_| 0.6),
                prospect_slots_used: slots_used,
                seller_rep_score: 0.30,
                buyer_rep_score,
                target_available: false,
                wage_headroom: None,
                expected_wage: 250_000,
            }
        }

        #[allow(clippy::too_many_arguments)]
        fn decide_with_ctx(
            rep: ReputationLevel,
            philosophy: ClubPhilosophy,
            budget: f64,
            fee: f64,
            balance: i64,
            age: u8,
            prospect: &ProspectSigningContext,
            request: &TransferRequest,
        ) -> TransferApproach {
            PipelineProcessor::determine_transfer_approach(
                &rep,
                budget,
                fee,
                Some(request),
                age,
                Self::summer(),
                balance,
                &philosophy,
                prospect,
            )
        }

        #[allow(clippy::too_many_arguments)]
        fn decide(
            rep: ReputationLevel,
            philosophy: ClubPhilosophy,
            budget: f64,
            fee: f64,
            balance: i64,
            age: u8,
            scout: Option<(u8, u8)>,
            slots_used: u8,
            request: &TransferRequest,
        ) -> TransferApproach {
            let prospect = Self::ctx(&rep, scout, slots_used);
            Self::decide_with_ctx(
                rep, philosophy, budget, fee, balance, age, &prospect, request,
            )
        }

        /// Elite Balanced buyer with everything in order — the baseline
        /// "should buy" configuration the negative tests perturb.
        #[allow(clippy::too_many_arguments)]
        fn elite_buy_with(
            balance: i64,
            fee: f64,
            age: u8,
            scout: Option<(u8, u8)>,
            slots_used: u8,
        ) -> TransferApproach {
            Self::decide(
                ReputationLevel::Elite,
                ClubPhilosophy::Balanced,
                20_000_000.0,
                fee,
                balance,
                age,
                scout,
                slots_used,
                &Self::development_request(),
            )
        }

        /// Elite Balanced baseline with a custom prospect context.
        fn elite_buy_with_ctx(prospect: &ProspectSigningContext) -> TransferApproach {
            Self::decide_with_ctx(
                ReputationLevel::Elite,
                ClubPhilosophy::Balanced,
                20_000_000.0,
                2_000_000.0,
                5_000_000,
                18,
                prospect,
                &Self::development_request(),
            )
        }
    }

    #[test]
    fn elite_club_buys_development_prospect_permanently() {
        assert_eq!(
            ApproachFixtures::elite_buy_with(5_000_000, 2_000_000.0, 18, Some((60, 90)), 0),
            TransferApproach::PermanentTransfer,
            "wealthy elite club must buy the prospect outright, not borrow him"
        );
    }

    #[test]
    fn continental_sign_to_compete_buys_prospect() {
        let approach = ApproachFixtures::decide(
            ReputationLevel::Continental,
            ClubPhilosophy::SignToCompete,
            15_000_000.0,
            3_000_000.0,
            8_000_000,
            19,
            Some((70, 95)),
            0,
            &ApproachFixtures::development_request(),
        );
        assert_eq!(
            approach,
            TransferApproach::PermanentTransfer,
            "wealthy compete-now giant runs a prospect-ownership desk"
        );
    }

    #[test]
    fn loan_focused_club_keeps_borrowing_prospects() {
        let approach = ApproachFixtures::decide(
            ReputationLevel::National,
            ClubPhilosophy::LoanFocused,
            10_000_000.0,
            2_000_000.0,
            5_000_000,
            18,
            Some((60, 90)),
            0,
            &ApproachFixtures::development_request(),
        );
        assert_eq!(approach, TransferApproach::Loan);
    }

    #[test]
    fn small_balanced_club_keeps_borrowing_prospects() {
        let approach = ApproachFixtures::decide(
            ReputationLevel::Regional,
            ClubPhilosophy::Balanced,
            3_000_000.0,
            500_000.0,
            1_000_000,
            18,
            Some((60, 90)),
            0,
            &ApproachFixtures::development_request(),
        );
        assert_eq!(
            approach,
            TransferApproach::Loan,
            "small clubs lack the profile for prospect ownership"
        );
    }

    #[test]
    fn negative_balance_blocks_prospect_purchase() {
        assert_eq!(
            ApproachFixtures::elite_buy_with(-1_000_000, 2_000_000.0, 18, Some((60, 90)), 0),
            TransferApproach::Loan,
            "no prospect shopping in the red"
        );
    }

    #[test]
    fn window_cap_forces_loan_after_enough_prospect_buys() {
        assert_eq!(
            ApproachFixtures::elite_buy_with(5_000_000, 2_000_000.0, 18, Some((60, 90)), 3),
            TransferApproach::Loan,
            "elite per-window prospect cap is 3 — the 4th must not be a purchase"
        );
    }

    #[test]
    fn missing_scout_dossier_forces_loan() {
        assert_eq!(
            ApproachFixtures::elite_buy_with(5_000_000, 2_000_000.0, 18, None, 0),
            TransferApproach::Loan,
            "no scouted potential estimate → no basis to commit a fee"
        );
    }

    #[test]
    fn thin_assessed_potential_gap_forces_loan() {
        assert_eq!(
            ApproachFixtures::elite_buy_with(5_000_000, 2_000_000.0, 18, Some((80, 86)), 0),
            TransferApproach::Loan,
            "scouts see no meaningful upside → borrow, don't buy"
        );
    }

    #[test]
    fn fee_exceeding_budget_headroom_forces_loan() {
        assert_eq!(
            ApproachFixtures::elite_buy_with(5_000_000, 19_000_000.0, 18, Some((60, 90)), 0),
            TransferApproach::Loan,
            "prospect buys are optional investments — fee must leave budget headroom"
        );
    }

    #[test]
    fn over_age_target_forces_loan() {
        assert_eq!(
            ApproachFixtures::elite_buy_with(5_000_000, 2_000_000.0, 24, Some((90, 110)), 0),
            TransferApproach::Loan,
            "development purchases target ≤21 only"
        );
    }

    #[test]
    fn loan_to_fill_squad_remains_loan_first_even_for_elite() {
        let approach = ApproachFixtures::decide(
            ReputationLevel::Elite,
            ClubPhilosophy::Balanced,
            20_000_000.0,
            1_000_000.0,
            5_000_000,
            24,
            Some((80, 90)),
            0,
            &ApproachFixtures::loan_fill_request(),
        );
        assert_eq!(
            approach,
            TransferApproach::Loan,
            "LoanToFillSquad must stay loan-first regardless of buyer wealth"
        );
    }

    #[test]
    fn low_scout_confidence_forces_loan() {
        let mut prospect = ApproachFixtures::ctx(&ReputationLevel::Elite, Some((60, 90)), 0);
        prospect.scout_confidence = Some(0.20);
        assert_eq!(
            ApproachFixtures::elite_buy_with_ctx(&prospect),
            TransferApproach::Loan,
            "a one-look dossier must not justify buying a teenager"
        );
    }

    #[test]
    fn peer_seller_without_gettable_signal_forces_loan() {
        let mut prospect = ApproachFixtures::ctx(&ReputationLevel::Elite, Some((60, 90)), 0);
        prospect.seller_rep_score = 0.88; // effectively a peer of the elite buyer
        prospect.target_available = false;
        assert_eq!(
            ApproachFixtures::elite_buy_with_ctx(&prospect),
            TransferApproach::Loan,
            "peers don't sell happy, playing prospects — no purchase attempt"
        );
    }

    #[test]
    fn peer_seller_with_listed_player_allows_purchase() {
        let mut prospect = ApproachFixtures::ctx(&ReputationLevel::Elite, Some((60, 90)), 0);
        prospect.seller_rep_score = 0.88;
        prospect.target_available = true; // listed / unhappy / fringe
        assert_eq!(
            ApproachFixtures::elite_buy_with_ctx(&prospect),
            TransferApproach::PermanentTransfer,
            "a gettable signal unlocks peer-club prospect purchases"
        );
    }

    #[test]
    fn exhausted_wage_headroom_forces_loan() {
        let mut prospect = ApproachFixtures::ctx(&ReputationLevel::Elite, Some((60, 90)), 0);
        prospect.wage_headroom = Some(100_000.0);
        prospect.expected_wage = 250_000;
        assert_eq!(
            ApproachFixtures::elite_buy_with_ctx(&prospect),
            TransferApproach::Loan,
            "the board's wage mandate must absorb the new contract too"
        );
    }
}

#[cfg(test)]
mod dev_pathway_cleanup_tests {
    use super::*;
    use crate::club::academy::ClubAcademy;
    use crate::club::player::builder::PlayerBuilder;
    use crate::competitions::global::GlobalCompetitions;
    use crate::continent::Continent;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::Location;
    use crate::shared::fullname::FullName;
    use crate::transfers::pipeline::{LoanOutCandidate, LoanOutReason, LoanOutStatus};
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, Country, PersonAttributes,
        Player, PlayerAttributes, PlayerCollection, PlayerPosition, PlayerPositionType,
        PlayerPositions, PlayerSkills, StaffCollection, Team, TeamCollection, TeamReputation,
        TeamType, TrainingSchedule,
    };
    use chrono::{NaiveDate, NaiveTime};

    /// Fixtures for the ownership-aware cleanup behaviour. Wrapped in a
    /// unit struct per the project's no-free-helpers convention.
    struct OwnershipFixtures;

    impl OwnershipFixtures {
        fn d(y: i32, m: u32, day: u32) -> NaiveDate {
            NaiveDate::from_ymd_opt(y, m, day).unwrap()
        }

        fn player(id: u32) -> Player {
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Dev".to_string(), format!("P{id}")))
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
                .player_attributes(PlayerAttributes::default())
                .build()
                .unwrap()
        }

        fn team(id: u32, club_id: u32, players: Vec<Player>) -> Team {
            Team::builder()
                .id(id)
                .league_id(Some(10))
                .club_id(club_id)
                .name(format!("Team {id}"))
                .slug(format!("team-{id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(5000, 5000, 5000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap()
        }

        fn club(id: u32, name: &str, teams: Vec<Team>) -> Club {
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

        fn dev_candidate(player_id: u32) -> LoanOutCandidate {
            LoanOutCandidate {
                player_id,
                reason: LoanOutReason::DevelopmentPathway,
                status: LoanOutStatus::Identified,
                loan_fee: 0.0,
            }
        }

        fn world(clubs: Vec<Club>) -> SimulatorData {
            let league = League::new(
                10,
                "L".to_string(),
                "league".to_string(),
                1,
                500,
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
            let country = Country::builder()
                .id(1)
                .code("en".to_string())
                .slug("england".to_string())
                .name("england".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(clubs)
                .build()
                .unwrap();
            let continent = Continent::new(1, "Europe".to_string(), vec![country], Vec::new());
            SimulatorData::new(
                Self::d(2026, 7, 5).and_hms_opt(12, 0, 0).unwrap(),
                vec![continent],
                GlobalCompetitions::new(Vec::new()),
            )
        }
    }

    /// The buyer staged a DevelopmentPathway candidate for a player it
    /// now rosters; another club holds a stale candidate for the same
    /// id. The post-transfer interest sweep must keep the owner's
    /// candidate (otherwise the same-window development loan dies in
    /// the same tick it's staged) and drop the stale one.
    #[test]
    fn loan_out_candidate_survives_cleanup_only_at_owning_club() {
        let player_id: u32 = 700;
        let mut owner = OwnershipFixtures::club(
            1,
            "Owner",
            vec![OwnershipFixtures::team(
                11,
                1,
                vec![OwnershipFixtures::player(player_id)],
            )],
        );
        owner
            .transfer_plan
            .loan_out_candidates
            .push(OwnershipFixtures::dev_candidate(player_id));

        let mut stale = OwnershipFixtures::club(2, "Stale", vec![]);
        stale
            .transfer_plan
            .loan_out_candidates
            .push(OwnershipFixtures::dev_candidate(player_id));

        let mut data = OwnershipFixtures::world(vec![owner, stale]);

        PipelineProcessor::cleanup_player_transfer_interest(&mut data, player_id);

        let country = data.country(1).unwrap();
        let owner = country.clubs.iter().find(|c| c.id == 1).unwrap();
        assert!(
            owner
                .transfer_plan
                .loan_out_candidates
                .iter()
                .any(|c| c.player_id == player_id && c.reason == LoanOutReason::DevelopmentPathway),
            "owning club's development-pathway candidate must survive the sweep"
        );
        let stale = country.clubs.iter().find(|c| c.id == 2).unwrap();
        assert!(
            stale
                .transfer_plan
                .loan_out_candidates
                .iter()
                .all(|c| c.player_id != player_id),
            "non-owning club's stale candidate must be dropped"
        );
    }
}

#[cfg(test)]
mod synthetic_listing_price_tests {
    use super::SyntheticListingPrice;
    use crate::shared::{Currency, CurrencyValue};

    fn usd(amount: f64) -> CurrencyValue {
        CurrencyValue {
            amount,
            currency: Currency::Usd,
        }
    }

    /// Regression #7: the synthetic listing backing an unsolicited approach
    /// must advertise the SELLER's asking price, never the buyer's
    /// (budget-capped) offer. Litvinov-like case: seller asks ~5.5M, the
    /// cash-poor suitor could only bid 340K — the listing must read 5.5M, so
    /// the offer ÷ asking ratio exposes the bid as the lowball it is rather
    /// than the old offer × 1.2 = 408K that made 340K look like a fair deal.
    #[test]
    fn synthetic_listing_uses_seller_asking_not_buyer_offer() {
        let seller_asking = usd(5_500_000.0);
        let listing = SyntheticListingPrice::for_unsolicited(&seller_asking);
        assert_eq!(
            listing.amount, 5_500_000.0,
            "synthetic asking must equal the seller's valuation, not a buyer-offer proxy"
        );

        // The old bug computed offer × 1.2 (≈408K against a 340K bid). Whatever
        // the buyer bids, the synthetic asking is independent of it.
        let buyer_lowball_x12 = 340_000.0 * 1.2;
        assert!(
            listing.amount > buyer_lowball_x12 * 5.0,
            "synthetic asking {} must not track the buyer's budget-capped offer ({})",
            listing.amount,
            buyer_lowball_x12
        );
    }
}
