use crate::transfers::deal::reason::TransferReasonBuilder;
use crate::transfers::market::window::MarketCadence;
use crate::transfers::scouting::recruitment::meeting::MeetingPass;
use crate::transfers::value::asking::AskingPrice;
use crate::transfers::view::player::PlayerView;
use chrono::NaiveDate;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use crate::SimulatorData;
use crate::club::player::transfer::FreeAgentBlockReason;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::ScoutMonitoringStatus;
use crate::transfers::TransferWindowManager;
use crate::transfers::deal::negotiation::NegotiationStatus;
use crate::transfers::deal::offer::{TransferClause, TransferOffer};
use crate::transfers::deal::reason::TransferReason;
use crate::transfers::gate::appraisal::PlayerStance;
use crate::transfers::gate::build::TransferPlausibilityBuilder;
use crate::transfers::gate::fit::{ForeignSlotCount, SquadRegistrationLimits};
use crate::transfers::gate::stance::{AvailabilityView, PlayerStanceBuilder, StanceInputs};
use crate::transfers::gate::{
    TransferMovePlausibility, TransferMoveStage, TransferPlausibilityEvaluator,
    TransferPlausibilityVerdict,
};
mod domestic;
mod foreign;

use crate::transfers::MarketMap;
use crate::transfers::market::{
    TransferListing, TransferListingOrigin, TransferListingStatus, TransferListingType,
};
use crate::transfers::pipeline::trace::TransferTrace;
use crate::transfers::pipeline::{ClubTransferPlan, DetailedScoutingReport};
use crate::transfers::pipeline::{
    ShortlistCandidateStatus, TransferApproach, TransferNeedPriority, TransferNeedReason,
    TransferRequest, TransferRequestStatus,
};
use crate::transfers::pool::FreeAgentBumpBatch;
use crate::transfers::scouting::recruitment::ScoutPlayerMonitoring;
use crate::transfers::squad::plan::{BriefTier, PlanningCadence};
use crate::transfers::value::upgrade::{DealValue, TargetBelief, UpgradeMath};
use crate::transfers::value::wage::BuyerLevelWage;
use crate::transfers::view::club::ClubView;
use crate::utils::FormattingUtils;
use crate::{
    Club, ClubPhilosophy, ClubTransferStrategy, Country, Person, Player, PlayerFieldPositionGroup,
    PlayerStatusType, ReputationLevel, StaffPosition, Team, TransferStrategyContext,
    WageCalculator,
};
use domestic::DomesticApproachPass;
use foreign::ForeignApproachPass;

/// How close to the asking price a buyer is willing to push.
///
/// Continuous in reputation, replacing the old bucketed `ReputationLevel`
/// cliff: a club's willingness scales smoothly with how established it is
/// and how big it is relative to the seller — a small club overreaching for
/// a giant's player stays disciplined; a giant dealing with a small club
/// can push hard because it can wear the premium.
pub(in crate::transfers) struct BuyingAggressiveness;

impl BuyingAggressiveness {
    /// The share of the asking price a buyer of `buying_score` standing
    /// will push to against a seller of `selling_score`.
    pub(in crate::transfers) fn from_rep(buying_score: f32, selling_score: f32) -> f32 {
        let base = 0.30 + 0.55 * buying_score.clamp(0.0, 1.0);
        let ratio = if selling_score > 0.01 {
            (buying_score / selling_score).clamp(0.4, 2.0)
        } else {
            1.2
        };
        let ratio_adj = (ratio - 1.0) * 0.06;
        (base + ratio_adj).clamp(0.25, 0.90)
    }
}

/// The buying club, as an approach reads it.
///
/// Half of the seam between "who is this deal for" and "what is the deal".
/// The offer construction below needs a dozen facts about the buyer and a
/// dozen about the target; passing them loose was what kept the decision
/// welded into an 800-line loop, and welded into the DOMESTIC one at that.
pub(in crate::transfers) struct ApproachBuyer<'a> {
    pub club: &'a Club,
    pub team: &'a Team,
    pub plan: &'a ClubTransferPlan,
    /// Continuous 0..1 standing — what the aggressiveness curve reads.
    pub rep_score: f32,
    pub league_reputation: u16,
    /// Squad-average current ability, the coarse quality bar the strategy
    /// values targets against.
    pub avg_ability: u8,
    pub budget: f64,
}

/// The target and the club he is being bought from, however the pass found
/// them — off the local roster, or resolved across a border.
///
/// This is the half that differs by reach, and the reason the decision can
/// be shared: once a target is described this way, the offer does not care
/// which country he was standing in.
pub(in crate::transfers) struct ApproachTarget<'a> {
    pub player: &'a Player,
    pub selling_club: &'a Club,
    pub selling_club_id: u32,
    pub selling_rep_score: f32,
    pub selling_league_reputation: u16,
    pub is_rival: bool,
    pub monitoring: Option<&'a ScoutPlayerMonitoring>,
    pub scouting_report: Option<&'a DetailedScoutingReport>,
}

/// What the buyer decided to do about this target.
pub(in crate::transfers) enum ApproachOutcome {
    /// Open a negotiation on these terms.
    Approach(Box<NegotiationAction>),
    /// The staged plausibility model refused at the last gate. The caller
    /// marks the shortlist candidate so the cursor advances instead of
    /// stalling on him.
    Refused,
}

/// The tick-level facts an approach is made against.
pub(in crate::transfers) struct ApproachContext<'a> {
    /// The BUYER's country: its calendar, its deadline, its market.
    pub buy_country: &'a Country,
    /// The SELLER's. The same object as `buy_country` for a domestic move —
    /// which is exactly why this used to be one field and two functions.
    pub sell_country: &'a Country,
    /// Geography between the two. `MarketMap::default()` for a domestic move:
    /// both the corridor and the buyer's knowledge of the market are 1.0 by
    /// construction, which is what an empty map yields.
    pub market_map: &'a MarketMap,
    /// The SELLING country's price level — what an asking price is quoted in.
    pub price_level: f32,
    pub date: NaiveDate,
    pub shortlist_request_id: u32,
    /// Buy, loan, or loan-with-option — the DoF's call, made upstream.
    pub approach: TransferApproach,
    pub is_loan: bool,
    pub has_option_to_buy: bool,
    pub is_prospect_purchase: bool,
}

/// The inputs on which the two reaches genuinely still differ.
///
/// Everything else about an approach is one implementation now. These are not
/// preferences — they are drift between two copies written months apart, and
/// **every one of them moves money**, so each is spelled out at both call sites
/// rather than quietly unified by the merge. Reconcile them one at a time, each
/// with its own census run; that is the whole reason they are a struct with
/// names instead of a diff nobody will read.
pub(in crate::transfers) struct ApproachDrift {
    /// Domestic caps the strategy and the opening fee at the shortlist's own
    /// allocation; the cross-border pass has always spent against the whole
    /// transfer budget.
    pub allocated_budget: f64,
    /// Domestic reads the buying team's market-value score straight; the
    /// cross-border pass substitutes `avg_ability × 100` when that score is 0.
    pub valuation_reputation: u16,
    /// Where the target sits on the shortlist. `None` cross-border, though
    /// the cross-border pass has had the shortlist in hand all along.
    pub shortlist_rank: Option<u8>,
    /// How many rivals are already bidding. `None` cross-border.
    pub competition_count: Option<u8>,
    /// A loan from a big seller carries a 30 % / 10-appearance fee
    /// domestically and has never carried one cross-border. See
    /// [`OfferClauses::attach_loan_appearance_fee`].
    pub loan_appearance_fee: bool,
    /// Cross-border falls back to a generic "Loan signing" / "Transfer
    /// signing" when neither a request motive nor a scout note exists;
    /// domestic lets the empty reason stand.
    pub generic_reason_fallback: bool,
    /// The shortlist's own fee estimate, which runs the staged realism model
    /// once here at the moment the offer is complete. `None` cross-border: that
    /// pass runs a STRICTER gate earlier — it has to, because the same
    /// assessment is where the seller-side facts it stages come from — so a
    /// second, weaker one here would only be able to disagree.
    pub final_gate_fee: Option<f64>,
}

/// Turning an agreed target into an offer.
///
/// This is the ~280 lines both reaches share: strategy, asking price, wage,
/// deal valuation, the man he would replace, the opening ratio, clauses,
/// reason, and the last plausibility gate. It used to live welded inside
/// the DOMESTIC loop, with the cross-border pass carrying its own copy and
/// a dozen comments saying "mirror the domestic path".
///
/// The body is unchanged from that original — the context structs are read
/// straight back into the names it already used, so the move carries no
/// rename risk.
pub(in crate::transfers) struct ApproachBuilder;

impl ApproachBuilder {
    pub(in crate::transfers) fn build(
        buyer: &ApproachBuyer<'_>,
        target: &ApproachTarget<'_>,
        request: Option<&TransferRequest>,
        ctx: &ApproachContext<'_>,
        drift: &ApproachDrift,
    ) -> ApproachOutcome {
        let (asking_price, actual_asking, mut offer) =
            Self::open_offer(buyer, target, request, ctx, drift);

        let (offered_annual_wage, deal, tier) = Self::price_to_buyer(
            buyer,
            target,
            request,
            ctx,
            drift,
            &actual_asking,
            &mut offer,
        );

        Self::assemble(
            buyer,
            target,
            request,
            ctx,
            drift,
            &asking_price,
            &actual_asking,
            offer,
            offered_annual_wage,
            deal,
            tier,
        )
    }

    /// The club's opening number, priced by its own transfer strategy against
    /// what the seller is asking — and, for a loan, against the rental a loan
    /// fee actually is rather than the asset price.
    fn open_offer(
        buyer: &ApproachBuyer<'_>,
        target: &ApproachTarget<'_>,
        request: Option<&TransferRequest>,
        ctx: &ApproachContext<'_>,
        drift: &ApproachDrift,
    ) -> (CurrencyValue, CurrencyValue, TransferOffer) {
        let club = buyer.club;
        let plan = buyer.plan;
        let buying_rep_score = buyer.rep_score;
        let buying_league_reputation = buyer.league_reputation;
        let avg_ability = buyer.avg_ability;
        let budget = buyer.budget;
        let price_level = ctx.price_level;
        let player = target.player;
        let selling_club = target.selling_club;
        let selling_rep_score = target.selling_rep_score;
        let player_id = player.id;
        let is_rival = target.is_rival;
        let monitoring = target.monitoring;
        let scouting_report = target.scouting_report;
        let buy_country = ctx.buy_country;
        let sell_country = ctx.sell_country;
        let date = ctx.date;
        let approach = &ctx.approach;
        let is_loan = ctx.is_loan;
        let has_option_to_buy = ctx.has_option_to_buy;

        let buying_aggressiveness =
            BuyingAggressiveness::from_rep(buying_rep_score, selling_rep_score);

        let allocated_for_move = drift.allocated_budget;
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
        )
        .with_valuation_reputation(drift.valuation_reputation);

        let asking_price = AskingPrice::calculate_asking_price(
            player,
            sell_country,
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
            Some(MeetingPass::build_board_dossier(
                plan,
                player_id,
                ctx.shortlist_request_id,
            ))
        } else {
            None
        };
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
            is_january: MarketCadence::is_mid_season_window_for(&buy_country.code, date),
            price_level,
            shortlist_rank: drift.shortlist_rank,
            competition_count: drift.competition_count,
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

        let offer =
            strategy.calculate_initial_offer_with_context(player, &actual_asking, &strategy_ctx);

        (asking_price, actual_asking, offer)
    }

    /// What this deal is worth to THIS buyer.
    ///
    /// The strategy prices the offer off the asking price and the budget; it
    /// has no way to say that the same player is worth three times as much to
    /// a club sitting on years of income as to a break-even one, and that
    /// asymmetry is the whole reason a market has ladders. [`UpgradeMath`]
    /// supplies the buyer's own ceiling — the fee at which the deal stops
    /// being worth doing — and the tier supplies how boldly it opens.
    ///
    /// Loans are left alone: a loan fee is a rental, not an asset purchase,
    /// and the upgrade model prices assets.
    #[allow(clippy::too_many_arguments)]
    fn price_to_buyer(
        buyer: &ApproachBuyer<'_>,
        target: &ApproachTarget<'_>,
        request: Option<&TransferRequest>,
        ctx: &ApproachContext<'_>,
        drift: &ApproachDrift,
        actual_asking: &CurrencyValue,
        offer: &mut TransferOffer,
    ) -> (u32, Option<DealValue>, BriefTier) {
        let club = buyer.club;
        let plan = buyer.plan;
        let buying_rep_score = buyer.rep_score;
        let buying_league_reputation = buyer.league_reputation;
        let player = target.player;
        let monitoring = target.monitoring;
        let scouting_report = target.scouting_report;
        let buy_country = ctx.buy_country;
        let date = ctx.date;
        let is_loan = ctx.is_loan;
        let allocated_for_move = drift.allocated_budget;

        // ── What this deal is worth to THIS buyer ───────────
        //
        // The strategy above prices the offer off the asking
        // price and the budget; it has no way to say that the
        // same player is worth three times as much to a club
        // sitting on years of income as to a break-even one, and
        // that asymmetry is the whole reason a market has
        // ladders. `UpgradeMath` supplies the buyer's own
        // ceiling — the fee at which the deal stops being worth
        // doing — and the tier supplies how boldly it opens.
        //
        // Loans are left alone: a loan fee is a rental, not an
        // asset purchase, and the upgrade model prices assets.
        //
        // The wage is priced first because the deal valuation
        // needs it: a fee is only half of what the buyer pays.
        // It reflects the ROLE the buyer signs the player into —
        // the same `ContractValuation` the player's personal-terms
        // reservation uses, so offer and demand share one wage
        // curve. The plain market wage (no squad-status premium)
        // sat structurally below a KeyPlayer/FirstTeamRegular
        // demand (market × 1.45 / 1.15), so personal terms opened
        // −18 to −5 and seldom converged. His current standing is
        // the best proxy for the role a suitor is buying — it
        // mirrors the reservation side's assumed status.
        //
        // The package the buyer built already carries this
        // figure (`PersonalTermsPackager` prices the same
        // curve for the same promise), and it is the one the
        // contract is installed on — so take it when it is
        // there rather than computing a second number that
        // can differ from the one he says yes to.
        let offered_annual_wage = offer
            .personal_terms
            .as_ref()
            .and_then(|t| t.annual_wage)
            .filter(|w| *w > 0)
            .unwrap_or_else(|| {
                BuyerLevelWage::evaluate(
                    player,
                    player.age(date),
                    buying_rep_score,
                    buying_league_reputation,
                    offer
                        .personal_terms
                        .as_ref()
                        .and_then(|t| t.squad_status_promise),
                )
            });

        let tier = request.map(|r| r.tier).unwrap_or(BriefTier::B);
        let deal = if is_loan {
            None
        } else {
            let group = player.position().position_group();
            let believed_level = monitoring
                .map(|m| m.current_assessed_ability)
                .or_else(|| scouting_report.map(|r| r.assessed_ability))
                .unwrap_or_else(|| PlayerView::position_evaluation_ability(player))
                as f32;
            let believed_ceiling = monitoring
                .map(|m| m.current_assessed_potential)
                .or_else(|| scouting_report.map(|r| r.assessed_potential))
                .unwrap_or(believed_level as u8) as f32;
            // The man he would REPLACE, not the best man in the
            // group. The brief knows which shirt this search is
            // for and who wears it; a club buying a second
            // centre-back measures him against its second
            // centre-back. Measured against the group's best,
            // every depth, succession and cover signing read as
            // a downgrade, was priced at nothing, and never had
            // its bid improved — the first census showed a
            // window's worth of ordinary business frozen at its
            // opening offers because of exactly that.
            let incumbent_level = request
                .and_then(|r| plan.brief.as_ref()?.slot_for(r.position))
                .map(|s| s.incumbent_level as f32)
                .unwrap_or_else(|| UpgradeMath::incumbent_level(club, group));
            UpgradeMath::priced(
                club,
                date,
                &TargetBelief {
                    group,
                    tier,
                    believed_level,
                    incumbent_level,
                    believed_ceiling,
                    age: player.age(date),
                    annual_wage: offered_annual_wage as f64,
                },
            )
        };

        // Open where the tier and the calendar say, not where
        // the budget alone does — and never above what the deal
        // is worth or what the club can fund. A club that opens
        // at 60 % of the ask for the signing meant to change its
        // season is not negotiating, it is wasting a window. A
        // request under negotiation is by definition still
        // unfilled, so the deadline premium reads the tier alone.
        if let Some(deal) = deal.as_ref() {
            let deadline = PlanningCadence::deadline_window(buy_country, date);
            let open_ratio = UpgradeMath::open_ratio(tier, deadline.days_left_fraction());
            let premium = deadline.premium_for(tier, true);
            let opening = actual_asking.amount * (open_ratio + premium);
            let capped = opening.min(deal.ceiling_fee).min(allocated_for_move);
            if capped > offer.base_fee.amount {
                offer.base_fee.amount = FormattingUtils::round_fee(capped);
            }
        }

        (offered_annual_wage, deal, tier)
    }

    /// The clauses, the negotiator, the reason, and the last plausibility
    /// check before a negotiation exists at all — a candidate refused here is
    /// marked unavailable and never gets a synthetic listing downstream.
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        buyer: &ApproachBuyer<'_>,
        target: &ApproachTarget<'_>,
        request: Option<&TransferRequest>,
        ctx: &ApproachContext<'_>,
        drift: &ApproachDrift,
        asking_price: &CurrencyValue,
        actual_asking: &CurrencyValue,
        mut offer: TransferOffer,
        offered_annual_wage: u32,
        deal: Option<DealValue>,
        tier: BriefTier,
    ) -> ApproachOutcome {
        let club = buyer.club;
        let team = buyer.team;
        let plan = buyer.plan;
        let buying_rep_score = buyer.rep_score;
        let buying_league_reputation = buyer.league_reputation;
        let player = target.player;
        let selling_club = target.selling_club;
        let selling_club_id = target.selling_club_id;
        let selling_rep_score = target.selling_rep_score;
        let selling_league_reputation = target.selling_league_reputation;
        let player_id = player.id;
        let is_rival = target.is_rival;
        let buy_country = ctx.buy_country;
        let sell_country = ctx.sell_country;
        let date = ctx.date;
        let is_loan = ctx.is_loan;
        let has_option_to_buy = ctx.has_option_to_buy;
        let is_prospect_purchase = ctx.is_prospect_purchase;

        OfferClauses::attach_prospect_sell_on(
            &mut offer,
            is_prospect_purchase,
            selling_rep_score,
            buying_rep_score,
        );

        OfferClauses::attach_loan_duration(&mut offer, is_loan);
        OfferClauses::attach_loan_option(&mut offer, has_option_to_buy, &asking_price);
        OfferClauses::attach_loan_appearance_fee(
            &mut offer,
            is_loan && drift.loan_appearance_fee,
            ClubView::get_club_reputation_level(sell_country, selling_club_id),
        );

        // Resolve negotiator staff and build reason
        let negotiator_staff_id = team.staffs.find_negotiator().map(|s| s.id);

        let scout_report = plan
            .scouting_reports
            .iter()
            .find(|r| r.player_id == player_id);

        let need_and_scout = TransferReasonBuilder::build_transfer_reason(request, scout_report);
        let reason = if drift.generic_reason_fallback && need_and_scout.is_empty() {
            if is_loan {
                TransferReason::key("signing_reason_loan")
            } else {
                TransferReason::key("signing_reason_transfer")
            }
        } else {
            need_and_scout
        };

        // Final plausibility check immediately before creating
        // the negotiation action. Rejected candidates are
        // marked unavailable here and a synthetic listing is
        // never created downstream — see Pass 2 for the
        // matching skip.
        if let Some(estimated_fee) = drift.final_gate_fee {
            let plausibility_inputs = TransferPlausibilityBuilder::from_global(
                buy_country,
                club,
                sell_country,
                selling_club,
                player,
                estimated_fee,
                is_loan,
                true, // unsolicited at the negotiation-entry point
                date,
                ctx.market_map,
            );
            if let TransferPlausibilityVerdict::HardReject(_reason) =
                TransferPlausibilityEvaluator::evaluate(&plausibility_inputs)
            {
                return ApproachOutcome::Refused;
            }
        }

        ApproachOutcome::Approach(Box::new(NegotiationAction {
            club_id: club.id,
            player_id,
            selling_club_id,
            offer,
            is_loan,
            has_option_to_buy,
            is_prospect_purchase,
            shortlist_request_id: ctx.shortlist_request_id,
            negotiator_staff_id,
            reason,
            player_name: player.full_name.to_string(),
            selling_club_name: selling_club.name.clone(),
            player_sold_from: player.sold_from.clone(),
            offered_annual_wage,
            buying_league_reputation,
            selling_league_reputation,
            player_stage_inclination: player.big_stage_inclination,
            buyer_ceiling_fee: deal.as_ref().map(|d| d.ceiling_fee),
            brief_tier: Some(tier),
            is_rival,
            seller_asking: actual_asking.clone(),
        }))
    }
}

/// The clauses an offer carries beyond its headline fee.
///
/// A loan from a big seller is priced partly in appearances — the parent
/// wants its prospect played, not parked — and a loan-with-option carries
/// the price the borrower may buy him at. A prospect bought outright
/// compensates the club that developed him with a share of the next sale.
/// Each is gated on the approach the buyer actually chose.
pub(in crate::transfers) struct OfferClauses;

impl OfferClauses {
    /// Sell-on share pledged to a clearly smaller selling side.
    const SELL_ON_SMALLER_SELLER: f32 = 0.15;
    /// The same when the seller is a peer rather than a feeder.
    const SELL_ON_PEER: f32 = 0.10;
    /// Below this share of the buyer's standing, the seller reads as the
    /// development side of the deal rather than a peer.
    const SMALLER_SELLER_BAR: f32 = 0.75;
    /// Months a loan runs for when the offer does not say otherwise.
    const DEFAULT_LOAN_MONTHS: u8 = 10;

    /// A prospect purchase compensates the development club with a sell-on
    /// share — bigger when the seller is clearly the smaller side. Skipped
    /// when the buyer's own strategy already pledged one.
    pub(in crate::transfers) fn attach_prospect_sell_on(
        offer: &mut TransferOffer,
        is_prospect_purchase: bool,
        seller_rep_score: f32,
        buyer_rep_score: f32,
    ) {
        let already_pledged = offer
            .clauses
            .iter()
            .any(|c| matches!(c, TransferClause::SellOnClause(_)));
        if !is_prospect_purchase || already_pledged {
            return;
        }
        let pct = if seller_rep_score < buyer_rep_score * Self::SMALLER_SELLER_BAR {
            Self::SELL_ON_SMALLER_SELLER
        } else {
            Self::SELL_ON_PEER
        };
        offer.clauses.push(TransferClause::SellOnClause(pct));
    }

    /// Share of the fee an Elite seller takes as an appearance fee, and the
    /// appearances it is measured over.
    const ELITE: (f64, u32) = (0.30, 10);
    /// The same for a Continental seller — a smaller cut over a longer run.
    const CONTINENTAL: (f64, u32) = (0.20, 15);
    /// What a loan option prices the eventual purchase at, against the
    /// seller's asking price.
    const OPTION_OF_ASKING: f64 = 0.7;

    /// How long a loan runs. The market reads it both for the history
    /// record and to bind the negotiation to a LOAN listing when the player
    /// is also transfer-listed — a loan bid anchored on the permanent
    /// asking price escalated toward the full valuation.
    pub(in crate::transfers) fn attach_loan_duration(offer: &mut TransferOffer, is_loan: bool) {
        if is_loan && offer.loan_duration_months.is_none() {
            offer.loan_duration_months = Some(Self::DEFAULT_LOAN_MONTHS);
        }
    }

    /// The price a loan-with-option lets the borrower buy him at.
    pub(in crate::transfers) fn attach_loan_option(
        offer: &mut TransferOffer,
        has_option_to_buy: bool,
        asking_price: &CurrencyValue,
    ) {
        if !has_option_to_buy {
            return;
        }
        let option_price = FormattingUtils::round_fee(asking_price.amount * Self::OPTION_OF_ASKING);
        offer
            .clauses
            .push(TransferClause::LoanOptionToBuy(CurrencyValue {
                amount: option_price,
                currency: Currency::Usd,
            }));
    }

    /// A big seller prices part of a loan in appearances — it wants its
    /// prospect played, not parked.
    ///
    /// **Domestic approaches only, today.** The cross-border pass attaches
    /// the sell-on, the option and the duration but has never attached
    /// this, so a foreign loan from an Elite seller carries no appearance
    /// fee where the identical domestic loan does. That asymmetry looks
    /// like drift rather than intent — the cross-border comments say
    /// "mirrors the domestic path" for every neighbouring clause — but
    /// closing it moves money, so it is recorded here and left for a
    /// census-gated commit of its own rather than folded into a move.
    pub(in crate::transfers) fn attach_loan_appearance_fee(
        offer: &mut TransferOffer,
        is_loan: bool,
        seller_level: ReputationLevel,
    ) {
        if !is_loan {
            return;
        }
        let terms = match seller_level {
            ReputationLevel::Elite => Some(Self::ELITE),
            ReputationLevel::Continental => Some(Self::CONTINENTAL),
            _ => None,
        };
        if let Some((share, appearances)) = terms {
            offer.clauses.push(TransferClause::AppearanceFee(
                CurrencyValue {
                    amount: FormattingUtils::round_fee(offer.base_fee.amount * share),
                    currency: Currency::Usd,
                },
                appearances,
            ));
        }
    }
}

/// Buyer-side context for the prospect buy-vs-loan decision. Bundles the
/// scouts' read on the target, the hoarding-cap usage, seller-vs-buyer
/// standing, and wage room — everything observable; the hidden biological
/// PA never feeds this.
pub(in crate::transfers) struct ProspectSigningContext {
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

pub(in crate::transfers) struct NegotiationAction {
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
    reason: TransferReason,
    player_name: String,
    selling_club_name: String,
    player_sold_from: Option<(u32, f64)>,
    offered_annual_wage: u32,
    buying_league_reputation: u16,
    /// The SELLER's league reputation. Paired with the buyer's so the
    /// personal-terms resolver can tell a genuine step up in stage from a
    /// sideways move — the difference a player chasing a bigger league
    /// cares about more than the badge on the shirt.
    selling_league_reputation: u16,
    /// The player's big-stage pull, staged for the resolver.
    player_stage_inclination: f32,
    /// The buyer's own ceiling for this deal and the tier of the request it
    /// answers — staged so the fee resolver's escalation has something to
    /// escalate TOWARD other than the seller's asking price. See
    /// [`crate::transfers::deal::negotiation::TransferNegotiation::buyer_ceiling_fee`].
    buyer_ceiling_fee: Option<f64>,
    brief_tier: Option<BriefTier>,
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

/// Foreigner-quota room for the buying clubs of ONE country, counted at
/// most once per club per pass.
///
/// `initiate_foreign_negotiations` walks a candidate list that can name the
/// same buyer many times, and the count is a full main-squad scan — cheap
/// once, wasteful per candidate.
#[derive(Default)]
struct ForeignRegistrationGuard {
    by_club: HashMap<u32, ForeignSlotCount>,
}

impl ForeignRegistrationGuard {
    /// Would this club be unable to register a player with this passport?
    fn would_block(
        &mut self,
        buying_country: &Country,
        buying_club_id: u32,
        candidate_country_id: u32,
    ) -> bool {
        let slots = match self.by_club.entry(buying_club_id) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let Some(club) = buying_country.clubs.iter().find(|c| c.id == buying_club_id)
                else {
                    return false;
                };
                let limits =
                    SquadRegistrationLimits::new(buying_country.id, &buying_country.regulations);
                *entry.insert(limits.count(club))
            }
        };
        slots.would_block(candidate_country_id)
    }
}

/// Opening the conversation: who the club approaches, at what price, and what the answer does to its plan.
pub struct ApproachPass;

impl ApproachPass {
    pub fn initiate_negotiations(country: &mut Country, date: NaiveDate) {
        DomesticApproachPass::run(country, date);
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
    /// Valuation context for the wage a BUYER offers a target.
    ///
    /// The role priced is the one the buyer is **promising**, not the one
    /// the player currently holds. Pricing off his current status meant a
    /// club offering a bench seat still paid for the key player he was
    /// leaving behind, and a club offering him the shirt paid for the
    /// backup he had become — and the player's own side, which reads the
    /// promise, then disagreed with the offer about what deal was on the
    /// table. `months_remaining` stays neutral: it only widens the
    /// acceptable band, never `expected_wage`, and the leverage that
    /// matters now lives in the appraisal's money weight.
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
        let is_january = MarketCadence::is_january_window(date);

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

        // Tier-driven approaches. The brief already decided how
        // transformative this signing is meant to be, and that decides
        // whether it is bought or borrowed: a club does not loan the man it
        // has built its window around, and it does not spend a fee on the
        // fourth centre-back when the loan market is open. Only cover
        // switches — the tiers that buy fall through to the affordability
        // and reason logic below exactly as before.
        if let Some(req) = request {
            if req.tier.prefers_loan() && !matches!(req.reason, TransferNeedReason::FormationGap) {
                return TransferApproach::Loan;
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
            PlayerView::find_player_in_country(country, player_id)
                .map(|p| p.position().position_group())
        } else {
            None
        };

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

            let (shortlist_matched, filled_group, manager_satisfaction_hit) =
                Self::resolve_shortlist_entry(plan, player_id, accepted);

            // The search is over: whatever the club had learned about failing
            // to fill this position stops applying the moment somebody signs
            // for it.
            if let Some(group) = filled_group {
                plan.clear_unmet_need(group);
            }

            // No shortlist candidate matched and the resolved deal was a
            // completed loan — i.e. a loan-scan signing. Mark the open
            // request in that position group fulfilled so it stops re-firing
            // and stacking further loans on an already-covered position.
            if !shortlist_matched {
                if let Some(group) = loan_filled_group {
                    plan.clear_unmet_need(group);
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
        // Membership-only sets probed once per free agent in the world, so
        // Fx hashing rather than the SipHash default — the same reason the
        // interest sweep below uses it.
        let offered: FxHashSet<u32> = batch.offered_ids.iter().copied().collect();
        let rejected: FxHashSet<u32> = batch.rejected_ids.iter().copied().collect();
        // Merge block reasons to the highest-ranked (closest-to-signing)
        // reason per player across every country that recorded one.
        let mut merged: FxHashMap<u32, FreeAgentBlockReason> = FxHashMap::default();
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

        // Every bump is player-local and the three sets are read-only, so
        // the pool walk fans out. It is the whole free-agent population once
        // a tick — tens of thousands of players by mid-save.
        data.free_agents.par_iter_mut().for_each(|player| {
            if offered.contains(&player.id) {
                player.on_offer_received(current_date);
            }
            if rejected.contains(&player.id) {
                player.on_offer_rejected(current_date);
            }
            if let Some(reason) = merged.get(&player.id) {
                player.on_market_blocked(current_date, *reason);
            }
        });
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

        // Losing bidders first: any club still holding a live negotiation
        // for a player someone else just signed loses the race here.
        // Resolve each through `on_negotiation_resolved` BEFORE the
        // clearing walk wipes the shortlist rows — that releases the
        // plan's negotiation slot, advances the shortlist, and re-opens
        // or abandons the request. The raw status flip at the end of the
        // sweep leaked all three, freezing the loser at its concurrency
        // cap for the rest of the window. Same-country losers were
        // already flipped Rejected (and resolved) at completion time, so
        // the Pending/Countered filter naturally selects only the
        // cross-country stragglers.
        // A country's own market is the only thing this touches, so the
        // world fans out. It matters more than the size of the loop
        // suggests: the sweep is called once per country that placed
        // somebody this tick, and a serial walk of every OTHER country
        // inside each of those calls is the shape that made the Phase-C
        // drain quadratic in the number of countries doing business.
        data.continents
            .par_iter_mut()
            .flat_map(|continent| continent.countries.par_iter_mut())
            .for_each(|country| {
                let losers: Vec<(u32, u32)> = country
                    .transfer_market
                    .negotiations
                    .values()
                    .filter(|n| {
                        signed.contains(&n.player_id)
                            && matches!(
                                n.status,
                                NegotiationStatus::Pending | NegotiationStatus::Countered
                            )
                    })
                    .map(|n| (n.buying_club_id, n.player_id))
                    .collect();
                for (club_id, player_id) in losers {
                    Self::on_negotiation_resolved(country, club_id, player_id, false);
                }
            });

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

                // A signed player's open listings close — EXCEPT a Loan
                // listing owned by the club that now rosters him: that's
                // the same-window development-loan listing his new club
                // just staged (young free signings stage during Phase A,
                // before this sweep runs), and completing it here killed
                // the dev loan before any borrower could see it.
                let own_loan_listing_pairs: Vec<(u32, u32)> = country
                    .clubs
                    .iter()
                    .flat_map(|c| {
                        c.teams
                            .teams
                            .iter()
                            .flat_map(|t| t.players.players.iter())
                            .map(move |p| (p.id, c.id))
                    })
                    .filter(|(pid, _)| signed.contains(pid))
                    .collect();
                for listing in country.transfer_market.listings.iter_mut() {
                    if signed.contains(&listing.player_id)
                        && listing.status != TransferListingStatus::Completed
                        && listing.status != TransferListingStatus::Cancelled
                    {
                        let is_new_owners_loan_listing = listing.listing_type
                            == TransferListingType::Loan
                            && own_loan_listing_pairs
                                .contains(&(listing.player_id, listing.club_id));
                        if !is_new_owners_loan_listing {
                            listing.status = listing_terminal.clone();
                        }
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
        ForeignApproachPass::run(data, country_id, date);
    }

    /// Mirror one resolved negotiation onto the shortlist that produced it:
    /// the candidate's own status, the request behind him, and what the
    /// outcome does to the manager's standing. Reports whether any shortlist
    /// claimed the player at all, and which position the club just filled.
    fn resolve_shortlist_entry(
        plan: &mut ClubTransferPlan,
        player_id: u32,
        accepted: bool,
    ) -> (bool, Option<PlayerFieldPositionGroup>, f32) {
        let mut shortlist_matched = false;
        let mut manager_satisfaction_hit: f32 = 0.0;

        // Position the club has just filled, if any. Collected inside the
        // shortlist loop and acted on after it, because the loop holds the
        // shortlists mutably for its whole body.
        let mut filled_group: Option<PlayerFieldPositionGroup> = None;

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
                        filled_group = Some(req.position.position_group());
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

                    // A need that was already Fulfilled (an FA instant
                    // signing or a parallel deal landed while this
                    // negotiation ran) or Abandoned (board veto) must
                    // not be resurrected by an unrelated failed bid —
                    // that re-opened filled needs and double-signed
                    // the position.
                    let request_live = plan
                        .transfer_requests
                        .iter()
                        .find(|r| r.id == shortlist.transfer_request_id)
                        .map(|r| {
                            r.status != TransferRequestStatus::Fulfilled
                                && r.status != TransferRequestStatus::Abandoned
                        })
                        .unwrap_or(false);

                    if request_live && shortlist.all_exhausted() {
                        if let Some(req) = plan
                            .transfer_requests
                            .iter_mut()
                            .find(|r| r.id == shortlist.transfer_request_id)
                        {
                            // A Critical need re-opens, and so now does one
                            // the club has failed at before: walking away
                            // from the weakest position in the side after a
                            // single unlucky shortlist is how a squad ends
                            // up carrying the same hole for years. The
                            // escalation count is capped, so this loop
                            // always terminates — once it tops out the
                            // request closes and the next squad evaluation
                            // picks the search back up, louder.
                            let reopens = req.priority == TransferNeedPriority::Critical
                                || req.escalation.reopens_on_exhaustion();
                            if reopens {
                                // Re-opened — but the repeated failure
                                // still stings.
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
                    } else if request_live {
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

        (shortlist_matched, filled_group, manager_satisfaction_hit)
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
    use crate::transfers::deal::negotiation::{NegotiationStatus, TransferNegotiation};
    use crate::transfers::deal::offer::TransferOffer;
    use crate::transfers::market::{TransferListing, TransferListingStatus, TransferListingType};
    use crate::transfers::pipeline::{
        ShortlistCandidate, ShortlistCandidateStatus, TransferShortlist,
    };
    use crate::transfers::scouting::recruitment::{
        ScoutMonitoringSource, ScoutMonitoringStatus, ScoutPlayerMonitoring,
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
                split_season: false,
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

        ApproachPass::cleanup_player_transfer_interest(&mut data, player_id);

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

        ApproachPass::cleanup_player_transfer_interest(&mut data, player_id);

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

        ApproachPass::cleanup_player_transfer_interest(&mut data, player_id);

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

        ApproachPass::on_negotiation_resolved(&mut country, 1, player_id, false);

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

        ApproachPass::on_negotiation_resolved(&mut country, 1, player_id, true);

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

        ApproachPass::cleanup_player_transfer_interest(&mut data, signed_id);

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
            ApproachPass::determine_transfer_approach(
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
    use crate::transfers::pipeline::{
        LoanDestinationPreference, LoanOutCandidate, LoanOutReason, LoanOutStatus,
    };
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
                preferred_destination: LoanDestinationPreference::Any,
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
                    split_season: false,
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

        ApproachPass::cleanup_player_transfer_interest(&mut data, player_id);

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
