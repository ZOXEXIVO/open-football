use super::execution::TransferExecution;
use super::free_agent_market_calc::BuyerRoleFit;
use super::free_agents::{EmergencySignedTerms, GlobalFreeAgentSigning};
use super::types::{
    DeferredTransfer, NegotiationData, PendingPlayerSignal, TransferActivitySummary,
    find_player_in_country, find_player_in_country_mut,
};
use crate::club::board::ownership::ClubBenefactor;
use crate::club::player::events::transfer_social::{
    TransferContinentalPath, TransferInterestSignal,
};
use crate::club::team::squad::{SquadAssetClass, SquadAssetProtection, SquadEvidenceContext};
use crate::country::result::CountryResult;
use crate::transfers::NegotiationStatus;
use crate::transfers::TransferListingStatus;
use crate::transfers::TransferListingType;
use crate::transfers::TransferRoutePolicy;
use crate::transfers::TransferWindowManager;
use crate::transfers::market::TransferListingOrigin;
use crate::transfers::negotiation::{
    NegotiationPhase, NegotiationRejectionReason, TransferNegotiation,
};
use crate::transfers::offer::{PersonalTermsOffer, PromisedSquadStatus, TransferClause};
use crate::transfers::pipeline::appraisal::{
    AppraisalConfig, OfferKind, OfferView, PlayerDisposition, PlayerOfferAppraisal, PlayerStance,
    TermsRefusalCause,
};
use crate::transfers::pipeline::appraisal_inputs::{
    AvailabilityView, OfferViewBuilder, PlayerStanceBuilder, StanceInputs,
};
use crate::transfers::pipeline::asset_ledger::ReplacementScarcity;
use crate::transfers::pipeline::auction::AuctionState;
use crate::transfers::pipeline::planning::{BriefTier, PlanningCadence};
use crate::transfers::pipeline::plausibility::{
    TransferPlausibilityBuilder, TransferPlausibilityEvaluator, TransferPlausibilityInputs,
    TransferPlausibilityVerdict,
};
use crate::transfers::pipeline::trace::TransferTrace;
use crate::transfers::pipeline::wage_power::{
    BuyerLevelWage, OwnerEnvelopeReservations, WagePower,
};
use crate::transfers::pipeline::{LoanOutReason, PipelineProcessor};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::{FloatUtils, FormattingUtils};
use crate::{
    Club, Country, Player, PlayerSquadStatus, PlayerStatusType, PlayerValueCalculator,
    TransferInterestSource, TransferInterestStage,
};
use chrono::NaiveDate;

/// What a fee is worth to the club being asked to sell it, measured
/// against the one number that decides these deals in real life: the
/// seller's own annual income.
///
/// Both resolvers used to read the fee only against the ASKING PRICE. That
/// is the right yardstick between peers — a Serie A club selling to
/// another Serie A club is haggling over a valuation — and it is the wrong
/// one when a giant calls a mid-table club abroad. A key man at asking
/// from a far bigger club engaged at 26% per approach and then went on a
/// rejection cooldown, no matter that the offer was worth half the
/// seller's yearly revenue. Real clubs at that level do not turn that
/// down; the decision leaves the coach's hands and becomes the chairman's.
///
/// So: a second axis. `windfall` is 0 for an ordinary fee and 1 for one
/// that changes the club's year, and it does exactly two things —
/// it forgives the *importance* damper on engagement, and it shaves the
/// premium the seller holds out for. It never touches the absolute
/// [`SellerFeeFloor`], the insult guard, or the rival penalty.
pub(crate) struct SellerWindfall {
    /// 0..1 — how transformative this fee is for the selling club.
    pub(crate) ratio: f32,
    /// 0..1 — how close the club is to its wage ceiling. A solvent club
    /// pressed against its wage budget has a reason of its own to bank a
    /// fee and free the shirt.
    pub(crate) wage_pressure: f32,
}

impl SellerWindfall {
    /// Fee ÷ annual income below which a bid is just money. Fifteen per
    /// cent of a year's revenue is a normal transfer for a selling club and
    /// changes nothing about how it behaves.
    const W0: f32 = 0.15;
    /// Span above `W0` over which the windfall saturates. At 60% of a
    /// year's income the sale is the club's whole financial story for the
    /// season.
    const W1: f32 = 0.45;
    /// Wage-bill-to-budget ratio at which a club is at its ceiling.
    const WAGE_CEILING: f32 = 0.95;
    /// Most the reservation premium eases on a full windfall.
    const RESERVATION_EASE: f64 = 0.25;
    /// Most it eases on wage-cap pressure alone.
    const WAGE_PRESSURE_EASE: f64 = 0.10;

    /// Score a fee against a seller's finances. `annual_income` is the
    /// annualised trailing figure (memory `club_finance_fm_rebuild` — never
    /// the raw trailing sum, which understates a young world by 12/N).
    pub(crate) fn of(fee: f64, annual_income: i64, wage_bill: i64, wage_budget: i64) -> Self {
        let ratio = if annual_income <= 0 {
            0.0
        } else {
            (((fee / annual_income as f64) as f32 - Self::W0) / Self::W1).clamp(0.0, 1.0)
        };
        // How far past the wage ceiling the club is running, saturating at
        // 10% over. Below the ceiling this is zero: a club with room has no
        // wage reason to sell anybody.
        let wage_pressure = if wage_budget <= 0 {
            0.0
        } else {
            let load = wage_bill as f32 / wage_budget as f32;
            ((load - Self::WAGE_CEILING) / 0.10).clamp(0.0, 1.0)
        };
        SellerWindfall {
            ratio,
            wage_pressure,
        }
    }

    /// Neutral windfall — used where the seller's finances are genuinely
    /// unavailable, so the model behaves exactly as it did before.
    pub(crate) fn none() -> Self {
        SellerWindfall {
            ratio: 0.0,
            wage_pressure: 0.0,
        }
    }

    /// How far the seller's reservation premium eases. Never lowers the
    /// absolute [`SellerFeeFloor`] — the floor guards the outcome, this
    /// only moves the premium the seller holds out for above asking.
    pub(crate) fn reservation_ease(&self) -> f64 {
        self.ratio as f64 * Self::RESERVATION_EASE
            + self.wage_pressure as f64 * Self::WAGE_PRESSURE_EASE
    }
}

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
    /// Saga beats for players who live in ANOTHER country — this pass
    /// runs inside the buying country's borrow, so the event is carried
    /// up and delivered in the serial Phase-C drain. Cross-border moves
    /// used to be silent to the player at every stage.
    pub(crate) player_signals: Vec<PendingPlayerSignal>,
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
            player_signals: Vec::new(),
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
                    let player_is_available =
                        super::types::find_player_in_country(country, n.player_id)
                            .map(|p| AvailabilityView::read(p, n.is_loan, listing_origin))
                            .map(|v| v.available_soft)
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
                        staged_reservation_wage: n.staged_reservation_wage,
                        buying_league_reputation: n.buying_league_reputation,
                        selling_league_reputation: n.selling_league_reputation,
                        player_stage_inclination: n.player_stage_inclination,
                        sell_on_percentage,
                        loan_future_fee: n.current_offer.loan_future_fee().map(
                            |(fee, obligation)| (fee.amount.max(0.0).round() as u32, obligation),
                        ),
                        personal_terms: n.current_offer.personal_terms.clone(),
                        foreign_terms_floor_blocked: n.foreign_terms_floor_blocked,
                        foreign_seller_importance: n.foreign_seller_importance,
                        foreign_seller_finances: n.foreign_seller_finances,
                        staged_stance: n.staged_stance,
                        staged_sporting_drop: n.staged_sporting_drop,
                    }
                }
                None => continue,
            };

            match neg_data.phase {
                NegotiationPhase::InitialApproach { .. } => {
                    Self::resolve_initial_approach(country, neg_id, &neg_data, date, &mut outcomes);
                }
                NegotiationPhase::ClubNegotiation { round, .. } => {
                    Self::resolve_club_negotiation(
                        country,
                        neg_id,
                        &neg_data,
                        round,
                        date,
                        &mut outcomes,
                    );
                }
                NegotiationPhase::PersonalTerms { round, .. } => {
                    Self::resolve_personal_terms(
                        country,
                        neg_id,
                        &neg_data,
                        round,
                        date,
                        &mut outcomes,
                    );
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

    /// Deliver one saga beat to the player, wherever he lives. A
    /// domestic player gets the full structured signal immediately; a
    /// foreign player's beat rides up on `outcomes.player_signals` and
    /// is delivered in the serial Phase-C drain (the player-dependent
    /// facts are resolved there). Pool free agents (selling_club_id 0)
    /// are skipped — their market state has its own machinery.
    fn notify_player_stage(
        country: &mut Country,
        outcomes: &mut NegotiationOutcomes,
        neg_data: &NegotiationData,
        stage: TransferInterestStage,
        source: TransferInterestSource,
        repeated_attention: bool,
    ) {
        match neg_data.selling_country_id {
            None => {
                if neg_data.selling_club_id == 0 {
                    return;
                }
                let signal = Self::build_interest_signal(
                    country,
                    neg_data,
                    stage,
                    source,
                    repeated_attention,
                );
                if let Some(sig) = signal {
                    if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
                        player.on_transfer_interest_signal(&sig);
                    }
                }
            }
            Some(selling_country_id) => {
                let interested_league_id = country
                    .clubs
                    .iter()
                    .find(|c| c.id == neg_data.buying_club_id)
                    .and_then(|c| c.teams.teams.first().and_then(|t| t.league_id));
                let hint = BuyerContinentalPathHint {
                    league_reputation: neg_data.buying_league_reputation,
                };
                outcomes.player_signals.push(PendingPlayerSignal {
                    player_id: neg_data.player_id,
                    selling_country_id,
                    selling_club_id: neg_data.selling_club_id,
                    stage,
                    source,
                    repeated_attention,
                    interested_club_id: neg_data.buying_club_id,
                    interested_league_id,
                    buyer_rep: neg_data.buying_rep,
                    seller_rep: neg_data.selling_rep,
                    buyer_league_rep: neg_data.buying_league_reputation,
                    buyer_country_id: country.id,
                    buyer_continent_id: country.continent_id,
                    buyer_has_continental_path: hint.is_on_path(),
                    buyer_competition_path: hint.competition_path(country.continent_id),
                });
            }
        }
    }

    /// Stamp the saga status the stage implies: `Bid` when the clubs
    /// agree a fee, `Trn` (replacing `Bid`) when the player agrees
    /// personal terms. Domestic players only — a foreign player's
    /// statuses cannot be reliably cleared from this country's borrow
    /// when the deal later dies, and a leaked `Trn` would quietly bench
    /// him forever (match selection rests near-sold players).
    fn set_saga_status(
        country: &mut Country,
        neg_data: &NegotiationData,
        status: PlayerStatusType,
        date: NaiveDate,
    ) {
        if neg_data.selling_country_id.is_some() || neg_data.selling_club_id == 0 {
            return;
        }
        if let Some(player) = find_player_in_country_mut(country, neg_data.player_id) {
            if status == PlayerStatusType::Trn {
                player.statuses.remove(PlayerStatusType::Bid);
            }
            player.statuses.add(date, status);
        }
    }

    /// Clear the saga statuses when a negotiation dies — unless another
    /// live negotiation for the same player is still past the fee-agreed
    /// line (two buyers can court one player; the survivor's saga keeps
    /// the badge). Pass `neg_id: 0` when the dying negotiation is
    /// already out of the market map (the expiry sweep).
    fn clear_saga_statuses(country: &mut Country, neg_id: u32, player_id: u32) {
        let another_live = country.transfer_market.negotiations.values().any(|n| {
            n.id != neg_id
                && n.player_id == player_id
                && matches!(
                    n.status,
                    NegotiationStatus::Pending | NegotiationStatus::Countered
                )
                && matches!(
                    n.phase,
                    NegotiationPhase::PersonalTerms { .. }
                        | NegotiationPhase::MedicalAndFinalization { .. }
                )
        });
        if another_live {
            return;
        }
        if let Some(player) = find_player_in_country_mut(country, player_id) {
            player.statuses.remove(PlayerStatusType::Bid);
            player.statuses.remove(PlayerStatusType::Trn);
        }
    }

    /// True when the selling club has the buying club flagged as a rival.
    /// Rivalry is an acceptance-chance friction (stronger at seller's end),
    /// not a hard block — a big enough bid or a player who forces the move
    /// still gets a deal through.
    /// How much of the BUYING club's spending its owner, rather than its
    /// revenue, is paying for — 0..1. The buyer is always in this country
    /// (the negotiation is resolved by its own pass), so this is readable
    /// on domestic and cross-border deals alike.
    fn buyer_benefactor(country: &Country, buying_club_id: u32) -> f32 {
        country
            .clubs
            .iter()
            .find(|c| c.id == buying_club_id)
            .map(|c| c.board.ownership.benefactor)
            .unwrap_or(0.0)
    }

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

    /// League-reputation gain that makes a buyer a genuine step up, and so
    /// someone the player's agent has already been talking to. Mirrors
    /// [`crate::transfers::pipeline::plausibility::TransferPlausibilityInputs::AGENT_MIN_STAGE_GAIN`]
    /// — the discovery side and the approach side must agree about what
    /// counts as a bigger stage.
    const AGENT_MIN_STAGE_GAIN: u16 = 1200;
    /// Engagement points a fully-committed player's own willingness adds to
    /// a step-up buyer's approach. Sized against the base unsolicited
    /// chance (35): at full pull it is the difference between a cold call
    /// and a conversation the seller was half expecting.
    const AGENT_LIFT: f32 = 20.0;

    /// The selling club's finances for the windfall model — read live for a
    /// domestic seller, off the staged snapshot for a foreign one (his club
    /// sits in another country's borrow while this negotiation resolves).
    fn seller_windfall(
        country: &Country,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) -> SellerWindfall {
        if let Some((income, wages, wage_budget)) = neg_data.foreign_seller_finances {
            return SellerWindfall::of(neg_data.offer_amount, income, wages, wage_budget);
        }
        let Some(club) = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)
        else {
            return SellerWindfall::none();
        };
        SellerWindfall::of(
            neg_data.offer_amount,
            club.finance.estimated_annual_income(date),
            club.teams
                .iter()
                .map(|t| t.get_annual_salary() as i64)
                .sum(),
            club.board
                .season_targets
                .as_ref()
                .map(|t| t.wage_budget.max(0) as i64)
                .unwrap_or(0),
        )
    }

    /// True when the selling club's own watchlist carries a name at the
    /// departing player's position that the fee on the table would fund.
    ///
    /// Reads the seller's board, not the world: a club is only comforted by
    /// a replacement it has actually identified. A club with no brief and no
    /// board has no successor by construction, which is the honest reading —
    /// it has not thought about it.
    fn seller_has_successor(country: &Country, neg_data: &NegotiationData) -> bool {
        let Some(selling_club) = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.selling_club_id)
        else {
            return false;
        };
        let Some(group) = selling_club
            .teams
            .main()
            .or_else(|| selling_club.teams.teams.first())
            .and_then(|team| {
                team.players
                    .players
                    .iter()
                    .find(|p| p.id == neg_data.player_id)
                    .map(|p| p.position().position_group())
            })
        else {
            return false;
        };
        selling_club
            .transfer_plan
            .watchlist
            .iter()
            .any(|e| e.position_group == group && e.estimated_value <= neg_data.offer_amount)
    }

    /// Live rival bids on one player, and the highest of them.
    ///
    /// Read from the negotiation table rather than tracked separately: the
    /// bids that are already in flight ARE the auction. The negotiation
    /// being resolved is excluded, so a buyer never bids against itself.
    fn auction_state(country: &Country, neg_id: u32, player_id: u32) -> AuctionState {
        let mut rivals = 0u32;
        let mut leading_bid = 0.0_f64;
        for n in country.transfer_market.negotiations.values() {
            if n.player_id != player_id || n.id == neg_id {
                continue;
            }
            if !matches!(
                n.status,
                NegotiationStatus::Pending | NegotiationStatus::Countered
            ) {
                continue;
            }
            rivals += 1;
            leading_bid = leading_bid.max(n.current_offer.base_fee.amount);
        }
        AuctionState {
            rivals,
            leading_bid,
        }
    }

    fn resolve_initial_approach(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        date: NaiveDate,
        outcomes: &mut NegotiationOutcomes,
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
        // Only the seller-acceptance delta is read up front now. The
        // importance-driven *premium* (the old `minimum_fee_multiplier`
        // wall) is no longer a reason to refuse to talk — it is negotiated
        // in the club-negotiation phase via the seller reservation. See the
        // note where `chance` is finalized below.
        let (seller_delta, importance_penalty) = match &plausibility {
            Some(TransferPlausibilityVerdict::Allow(adj)) => {
                (adj.seller_acceptance_delta, adj.importance_penalty)
            }
            _ => (0.0_f32, 0.0_f32),
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

        // NOTE: the absolute SellerFeeFloor is deliberately NOT enforced
        // here any more. A below-floor OPENING bid is what the
        // club-negotiation escalation rounds exist to fix — killing the
        // whole approach on the first number meant a buyer who opened at
        // 60% of value but could afford 85% never reached round two. The
        // floor still owns the outcome: `resolve_club_negotiation` refuses
        // to ACCEPT any bid below it, steers the buyer's escalation target
        // up to it, and terminally rejects on the final round if the bid
        // still falls short — so no deal can ever CLOSE below the floor
        // (the Litvinov 340K protection is intact, one phase later).

        // Important players are harder to prise away, but the seller no
        // longer refuses to TALK up front over an opening bid below a
        // replacement-value premium. That premium is settled in the
        // club-negotiation phase (seller reservation + buyer escalation),
        // and the absolute lower bound is already enforced above by the
        // `ratio < 0.45` insult guard and `SellerFeeFloor`. The importance
        // friction now lives entirely in `seller_delta`, which lowers the
        // engagement chance — so a happy key man still rarely moves, while
        // a genuinely bigger, determined suitor (or a player who has
        // outgrown his club and reads as available) can open talks and pay
        // the premium. This is what unblocked permanent moves for
        // established players: the old wall demanded an offer above the
        // buyer's own maximum, so no bid could ever clear it.
        chance += seller_delta;

        // …but "he is our key man" is the COACH's answer. Whether the club
        // says no is the chairman's, and his answer turns on what the money
        // does to the club's year. A bid worth a large slice of the
        // seller's annual income gives back the importance damper in
        // proportion to how transformative it is — which is exactly the
        // decision that was missing when a mid-table foreign club refused
        // a giant at asking price and then went on a rejection cooldown.
        // Nothing else is forgiven: the rival penalty, the insult guard and
        // the absolute fee floor are untouched.
        let windfall = Self::seller_windfall(country, neg_data, date);
        chance += importance_penalty * windfall.ratio;

        // The agent channel. A player drawn to a bigger stage is quietly
        // circulated, and a club that can offer him one finds the seller
        // readier to talk — this is what turns a cold 26% call into a real
        // conversation. Step-up buyers only: a lateral or downward move
        // gets nothing, so the reverse flow is untouched.
        let stage_gain = neg_data
            .buying_league_reputation
            .saturating_sub(neg_data.selling_league_reputation);
        let agent_led = stage_gain >= Self::AGENT_MIN_STAGE_GAIN
            && neg_data.player_stage_inclination
                >= TransferPlausibilityInputs::AGENT_INCLINATION_BAR;
        if stage_gain >= Self::AGENT_MIN_STAGE_GAIN {
            chance += neg_data.player_stage_inclination.clamp(0.0, 1.0) * Self::AGENT_LIFT;
        }
        if agent_led {
            country.transfer_market.story.agent_led_approaches += 1;
        }

        if TransferTrace::is(neg_data.player_id) {
            // Seller-side economics, the half of the funnel that leaves no
            // trace anywhere else: what the club thinks he is, what the fee
            // is worth to it, and how close it is to its wage ceiling.
            let seller_line = country
                .clubs
                .iter()
                .find(|c| c.id == neg_data.selling_club_id)
                .map(|club| {
                    let asset = find_player_in_country(country, neg_data.player_id)
                        .map(|p| SquadAssetProtection::classify(p, club, date))
                        .map(|c| c.label())
                        .unwrap_or("unknown");
                    let income = club.finance.estimated_annual_income(date);
                    let wages: i64 = club
                        .teams
                        .iter()
                        .map(|t| t.get_annual_salary() as i64)
                        .sum();
                    format!(
                        "club={} asset={asset} income={income} wages={wages} \
                         fee/income={:.2}",
                        club.name,
                        if income > 0 {
                            neg_data.offer_amount / income as f64
                        } else {
                            0.0
                        },
                    )
                })
                .unwrap_or_else(|| {
                    let (income, wages, budget) =
                        neg_data.foreign_seller_finances.unwrap_or((0, 0, 0));
                    format!(
                        "club={} (foreign) income={income} wages={wages} wage_budget={budget} \
                         fee/income={:.2}",
                        neg_data.selling_club_name,
                        if income > 0 {
                            neg_data.offer_amount / income as f64
                        } else {
                            0.0
                        },
                    )
                });
            TransferTrace::line(neg_data.player_id, "seller", seller_line);
            TransferTrace::line(
                neg_data.player_id,
                "approach",
                format!(
                    "buyer={} seller={} fee={:.0} asking={:.0} ratio={:.2} \
                     seller_delta={:.1} importance_give_back={:.1} windfall={:.2} \
                     wage_pressure={:.2} pull={:.2} stage_gain={}",
                    neg_data.buying_club_id,
                    neg_data.selling_club_id,
                    neg_data.offer_amount,
                    neg_data.asking_price,
                    ratio,
                    seller_delta,
                    importance_penalty * windfall.ratio,
                    windfall.ratio,
                    windfall.wage_pressure,
                    neg_data.player_stage_inclination,
                    stage_gain,
                ),
            );
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

        // Competition: when several buyers are bidding for the same player
        // the seller has leverage and is more inclined to engage. What
        // competition mostly moves, though, is the FEE, not whether the
        // seller picks up the phone — that half lives in
        // [`AuctionState::floor`], read by the fee resolver below. Here it
        // is only the engagement lift, saturating rather than compounding.
        let auction = Self::auction_state(country, neg_id, neg_data.player_id);
        chance += auction.seller_leverage();

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
            // …and so is a buyer whose money its own revenue cannot
            // explain. "Clearly bigger" is not only reputation: a club
            // sells to money it would not sell to a rival for, which is
            // the whole reason a mid-table side lets a starter go to the
            // Gulf and not to the team above it (L5).
            //
            // Continuous in the ratio, not a step at the state-backed bar:
            // a 0.49 benefactor and a 0.51 one are the same club, and a
            // cliff there is exactly the kind of threshold the whole model
            // exists to avoid.
            let benefactor = Self::buyer_benefactor(country, neg_data.buying_club_id);
            rival_penalty -= 12.0 * (benefactor / ClubBenefactor::STATE_BACKED_BAR).clamp(0.0, 1.0);
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
            // Foreign players get the same beat via the Phase-C drain.
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::ConcreteInterest,
                TransferInterestSource::ConfirmedApproach,
                false,
            );
        } else {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation
                    .reject_with_reason(NegotiationRejectionReason::SellerRefusedToNegotiate);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            // The target feels the rejection if it was a real chance.
            // Routed through the structured interest funnel so the
            // headline carries who rejected what and the player's reaction
            // (frustration / leverage / loyalty) lands with context.
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::BidRejected,
                TransferInterestSource::RejectedBid,
                false,
            );
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
        outcomes: &mut NegotiationOutcomes,
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
            // The clause fixes the fee — the clubs have effectively
            // agreed and talks with the player are next. Same beat as a
            // negotiated fee agreement below.
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::NegotiationsOpened,
                TransferInterestSource::ClubBriefing,
                false,
            );
            Self::set_saga_status(country, neg_data, PlayerStatusType::Bid, date);
            return;
        }

        // Absolute seller fee floor. The seller never ACCEPTS a bid below it
        // — but a below-floor bid mid-rounds is not an instant walk-away
        // either: the escalation path below steers the buyer's target up to
        // the floor, and only the FINAL round terminally rejects a bid that
        // still falls short. A triggered release clause above already
        // short-circuited, so a forced sale is never blocked.
        let floor = SellerFeeFloor::for_permanent_domestic(country, neg_data, date);
        let below_floor = floor
            .as_ref()
            .map(|f| neg_data.offer_amount < f.min_fee)
            .unwrap_or(false);

        // Peeked (not reserved) buyer transfer budget, so an escalation the
        // buyer could never fund fails fast here instead of collapsing at
        // the medical after weeks of sim time and a held shortlist slot.
        let buyer_transfer_budget: Option<f64> = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.buying_club_id)
            .and_then(|c| {
                c.finance
                    .transfer_budget
                    .as_ref()
                    .map(|b| b.amount.max(0.0))
            });

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

        // A long-unsold genuine listing erodes the seller's stance on the
        // same clock as `SellerFeeFloor::erode_for_listing_age`: with every
        // unsold month the club wants the wage off the books more than it
        // wants a premium, so its acceptance band walks down toward the
        // (itself eroding) absolute fee floor instead of holding a price
        // the market has already refused for half a season.
        if !neg_data.is_loan {
            if let Some(days_listed) = Self::seller_listing_age_days(country, neg_data, date) {
                let t = (days_listed as f64 / SellerFeeFloor::FLOOR_EROSION_DAYS).clamp(0.0, 1.0);
                seller_reservation -= t * 0.20;
            }
        }

        let urgency = Self::deadline_urgency_for(country, date) as f64;
        if urgency > 0.0 && importance < 0.75 {
            seller_reservation -= urgency * 0.10;
        }

        let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
        if rep_diff > 0.15 {
            // A clearly bigger suitor erodes the seller's leverage — the
            // wider the reputation gap, the less a small club can hold a
            // coveted player. This was previously cut off entirely for key
            // men (importance >= 0.85), which left a small club's star
            // unsellable to any giant; now the discount is softened at high
            // importance, not removed, so the standout-at-a-minnow → giant
            // move can actually clear the reservation.
            let gap = ((rep_diff - 0.15) * 0.20).min(0.11) as f64;
            let softener = if importance < 0.85 { 1.0 } else { 0.5 };
            seller_reservation -= (0.04 + gap) * softener;
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

        // A seller who stays at the table softens across rounds — real
        // negotiations converge from BOTH sides, not just the buyer bidding
        // up. Only the premium ABOVE asking concedes (never below 1.0, so the
        // absolute floor is still owned by `SellerFeeFloor`), and an already
        // available player (reservation <= 1.0) is untouched. Without this the
        // buyer's escalation could never meet a coveted player's static
        // premium and every such deal timed out in the sub-acceptance band.
        if round > 1 && seller_reservation > 1.0 {
            let concession = 0.05 * (round.saturating_sub(1) as f64);
            seller_reservation = (seller_reservation - concession).max(1.0);
        }

        // What the fee does to the seller's year. A club holds out for a
        // premium over asking because it does not need the money; one being
        // offered a large fraction of its annual revenue does. The second
        // term is the wage ceiling: a solvent club already at its wage
        // budget banks the fee AND frees the shirt, so it holds out less
        // for its highest earners. Neither eases the absolute
        // `SellerFeeFloor` — that still refuses the acceptance below, and
        // the clamp keeps the reservation in the same band it always had.
        let windfall = Self::seller_windfall(country, neg_data, date);
        seller_reservation -= windfall.reservation_ease();

        // Can the seller replace him with the money? A fee is only worth
        // taking if it buys a successor, and a club with nobody on its own
        // board to spend it on holds out — harder the later in the window it
        // gets, because a sale it cannot reinvest is a hole it carries all
        // season. This is also one of the two things that bound the drain:
        // without it, a buyer whose marginal dollar is worth a third of the
        // seller's could strip an exporting league bare.
        //
        // Foreign sellers live in another country's borrow and their board
        // cannot be read from here, so the term is simply absent for them —
        // it never invents a premium it cannot justify.
        //
        // Absent too for a player the club has ALREADY decided to move on:
        // listed, asking to leave, not needed, or quietly marketed on its own
        // ledger. Holding out for a successor is what a club does when it is
        // asked for a man it wants to keep; a decision to sell is a decision
        // that the hole is acceptable. The first census applied the term to
        // every domestic sale and priced ordinary business out of reach.
        if neg_data.selling_country_id.is_none() && !neg_data.is_loan {
            let marketed = country
                .clubs
                .iter()
                .find(|c| c.id == neg_data.selling_club_id)
                .map(|c| c.transfer_plan.is_marketed(neg_data.player_id))
                .unwrap_or(false);
            if !neg_data.player_is_available && !marketed {
                let has_successor = Self::seller_has_successor(country, neg_data);
                seller_reservation += ReplacementScarcity::lift(
                    has_successor,
                    importance,
                    PlanningCadence::deadline_window(country, date).pressure(),
                );
            }
        }

        seller_reservation = seller_reservation.clamp(0.55, 1.55);

        if TransferTrace::is(neg_data.player_id) {
            TransferTrace::line(
                neg_data.player_id,
                "fee",
                format!(
                    "round={round} buyer={} fee={:.0} asking={:.0} ratio={:.2} \
                     importance={:.2} reservation={:.2} windfall={:.2} ease={:.3} \
                     below_floor={below_floor}",
                    neg_data.buying_club_id,
                    neg_data.offer_amount,
                    neg_data.asking_price,
                    ratio,
                    importance,
                    seller_reservation,
                    windfall.ratio,
                    windfall.reservation_ease(),
                ),
            );
        }

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

        // What the market around this player is doing, and how close the
        // window is to shutting. Both are read once here — the acceptance
        // roll below and the buyer's escalation further down share them.
        let auction = Self::auction_state(country, neg_id, neg_data.player_id);
        let deadline = PlanningCadence::deadline_window(country, date);

        // In a contested sale the seller is not choosing between this bid and
        // nothing: it is choosing between this bid and the best one on the
        // table. A bid behind the leader is not the one that gets taken, and
        // in the last days — when there is no time left for another round —
        // the leader is taken rather than held out on.
        if auction.is_contested() {
            if auction.leads_with(neg_data.offer_amount) {
                chance += AuctionState::LEADER_BONUS * (1.0 + deadline.pressure());
            } else {
                chance *= AuctionState::TRAILING_SHARE;
            }
        }

        chance = chance.clamp(5.0, 95.0);
        let roll = FloatUtils::random(0.0, 100.0);

        // A below-floor bid can never be accepted, whatever the roll — the
        // floor guards the outcome while the rounds guard the process.
        if !below_floor && roll < chance {
            // Story counters: the beats that tell a live market from a
            // procurement queue. Diagnostics only — a layer whose counter
            // stays at zero has not landed, whatever the move totals say.
            if !neg_data.is_loan {
                let marketed = country
                    .clubs
                    .iter()
                    .find(|c| c.id == neg_data.selling_club_id)
                    .map(|c| c.transfer_plan.is_marketed(neg_data.player_id))
                    .unwrap_or(false);
                let story = &mut country.transfer_market.story;
                if auction.is_contested() {
                    story.contested_agreements += 1;
                }
                if deadline.is_deadline_week() {
                    story.deadline_agreements += 1;
                }
                if marketed {
                    story.sell_list_conversions += 1;
                }
            }
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.advance_to_personal_terms(date);
            }
            // Fee agreed — the single biggest formerly-silent beat: the
            // clubs have shaken hands and personal terms open. The `Bid`
            // badge makes the accepted bid visible AND lets match
            // selection start protecting a near-sold asset.
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::NegotiationsOpened,
                TransferInterestSource::ClubBriefing,
                false,
            );
            Self::set_saga_status(country, neg_data, PlayerStatusType::Bid, date);
        } else if round < 3 {
            // Buyer escalation closes 45% of the remaining gap per round at
            // the start of the window, up to ~70% in the final panic days.
            // The previous offer is archived to `counter_offers` so the
            // negotiation history is auditable (who bid what, when).
            // Escalate toward the price the seller will actually accept
            // (asking × his reservation), not merely the asking price.
            // A key man not pushing to leave commands a premium ABOVE
            // asking; aiming only at asking left every such deal short
            // of the reservation band, so it timed out. Clamped to
            // [1.0, 1.55]×asking: never below asking (so the working
            // available-player path is unchanged — its reservation is
            // <= 1.0 and clamps to 1.0), and up to the same 1.55 ceiling
            // the seller's reservation itself clamps to, so the buyer can
            // actually reach a coveted player's premium instead of topping
            // out below it. The target is additionally lifted to the
            // seller's absolute fee floor, so a lowball opening converges
            // toward a closable number instead of orbiting below it.
            // The asking price is only a valid escalation anchor when the
            // bound listing sells the same kind of deal. `start_negotiation`
            // binds to the first open listing for the player regardless of
            // type, so a LOAN bid on a transfer-listed player used to
            // escalate toward the full PERMANENT ask — loan fees converging
            // on half the player's market value (the loan-fee-anchor bug
            // reborn one phase later). A mismatched listing falls back to
            // the modest own-offer escalation path instead.
            let listing_matches_deal_type = country
                .transfer_market
                .negotiations
                .get(&neg_id)
                .and_then(|n| country.transfer_market.listings.get(n.listing_id as usize))
                .map(|l| {
                    if neg_data.is_loan {
                        l.listing_type == TransferListingType::Loan
                    } else {
                        l.listing_type != TransferListingType::Loan
                    }
                })
                .unwrap_or(true);

            let mut buyer_walks = false;
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                let reservation_mult = seller_reservation.clamp(1.0, 1.55);
                let mut target = if neg_data.asking_price > 0.0 && listing_matches_deal_type {
                    neg_data.asking_price * reservation_mult
                } else {
                    negotiation.current_offer.base_fee.amount * 1.15
                };
                if let Some(f) = &floor {
                    target = target.max(f.min_fee);
                }
                // What the buyer would have reached for before any of the
                // market layers existed: the seller's reservation, inside
                // its budget. This is the calibrated baseline reach and the
                // floor of everything below.
                let legacy_target = target;

                // ── The auction ─────────────────────────────────────
                //
                // Somebody else is already in front. A buyer that wants him
                // has to clear the leading bid by a real margin or it is
                // simply not in the conversation — and the one whose own
                // valuation runs out first stops raising and drops away.
                // This is what turns rivalry into money; the seller-side
                // engagement lift alone never moved a fee.
                if auction.is_contested() {
                    target = target.max(auction.floor());
                }

                // ── The deadline ────────────────────────────────────
                //
                // A club whose season plan still has a hole in it pays over
                // the odds in the last days. Cover does not: there is
                // always another body, and the loan market is open.
                let tier = negotiation.brief_tier.unwrap_or(BriefTier::B);
                let premium = deadline.premium_for(tier, true);
                if premium > 0.0 && neg_data.asking_price > 0.0 {
                    target += neg_data.asking_price * premium;
                }

                // ── The buyer's own number ──────────────────────────
                //
                // Everything above says what it would TAKE to win him.
                // This says what he is worth to this buyer. A club walks
                // away from a bidding war at its own ceiling, which is why
                // two clubs chasing one player do not converge on the same
                // fee — the richer one, whose marginal dollar is worth less,
                // simply has further to go.
                //
                // The ceiling caps only the EXTENSION above the legacy
                // reach — the auction floor and the deadline premium —
                // never the reach itself. Three censuses showed why: with
                // this market's fees an order of magnitude below its wages,
                // the deal value nets wages off a small sporting benefit and
                // comes out under the seller's price for exactly the deals
                // real clubs do (a strong-league side buying an elite club's
                // surplus, a mid-rich elite side buying a modest upgrade),
                // and those cells fell 40 % below HEAD. Until the fee scale
                // is reconciled the ceiling is not a trustworthy walk-away
                // price below the calibrated baseline, so it does not act
                // there. Where it does act — who keeps raising once a
                // bidding war has pushed the price past the ask — is the
                // half of the story it was built for.
                if let Some(ceiling) = negotiation.buyer_ceiling_fee {
                    target = target.min(ceiling.max(legacy_target));
                }

                let escalation = 0.45 + urgency * 0.25;
                let current_amount = negotiation.current_offer.base_fee.amount;
                let mut new_amount = FormattingUtils::round_fee(
                    current_amount + (target - current_amount).max(0.0) * escalation,
                );
                // Cap the escalated headline at what the buyer can actually
                // fund up front (installment tranches settle later from the
                // balance, so they extend the affordable headline). When the
                // BUDGET is what stops the bid improving, the buyer walks
                // now — the same refusal `reserve_transfer_budget` would
                // deliver at the medical, minus the wasted weeks. A bid that
                // already meets the target simply re-enters the next round
                // unchanged (the seller may yet say yes), as before.
                let mut budget_blocked = false;
                if let Some(available) = buyer_transfer_budget {
                    let deferred = new_amount
                        - TransferExecution::upfront_after_installments(
                            new_amount,
                            neg_data.selling_country_id.is_some(),
                            &negotiation.current_offer.clauses,
                        );
                    let max_affordable = FormattingUtils::round_fee(available + deferred.max(0.0));
                    if max_affordable < new_amount {
                        new_amount = max_affordable;
                        budget_blocked = true;
                    }
                }
                if budget_blocked && new_amount <= current_amount {
                    buyer_walks = true;
                } else {
                    let mut escalated = negotiation.current_offer.clone();
                    escalated.base_fee.amount = new_amount.max(current_amount);
                    escalated.offered_date = date;
                    negotiation.counter_offer(escalated);
                    negotiation.advance_club_negotiation_round(date);
                }
            }
            if buyer_walks {
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.reject_with_reason(NegotiationRejectionReason::AskingPriceTooHigh);
                }
                Self::reopen_listing_for_player(country, neg_data.player_id);
                // The suitor priced himself out and walked — the rumour
                // the player had been hearing goes quiet.
                Self::notify_player_stage(
                    country,
                    outcomes,
                    neg_data,
                    TransferInterestStage::InterestCooled,
                    TransferInterestSource::ClubBriefing,
                    false,
                );
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    false,
                );
            }
        } else {
            // Final round: a bid still under the seller's absolute floor is
            // rejected with the floor's own reason (PlayerTooImportant for a
            // core man), everything else as a plain price failure.
            let reason = floor
                .as_ref()
                .filter(|f| neg_data.offer_amount < f.min_fee)
                .map(|f| f.reason.clone())
                .unwrap_or(NegotiationRejectionReason::AskingPriceTooHigh);
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                negotiation.reject_with_reason(reason);
            }
            Self::reopen_listing_for_player(country, neg_data.player_id);
            // Final-round rejection — the buying club really did pursue
            // and the selling club still said no. Routed through the
            // structured interest funnel so the headline can name the
            // interested club and surface the player's reaction
            // (frustrated / contract-leverage / loyal).
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::BidRejected,
                TransferInterestSource::RejectedBid,
                true,
            );
            PipelineProcessor::on_negotiation_resolved(
                country,
                neg_data.buying_club_id,
                neg_data.player_id,
                false,
            );
        }
    }

    /// The player's side of the deal — [`PlayerOfferAppraisal`], once.
    ///
    /// This used to be an additive pile of roughly twenty hand-set bumps
    /// rolled once against a uniform, with a hard willingness floor in
    /// front of it and a separate reservation-wage ladder behind it. A
    /// player whose wage demand was already met got ONE roll, so a
    /// 60-point deal died four times in ten on luck alone; a Gulf club
    /// offering a Premier League star four times his money was refused by
    /// a −110-per-prestige-point wall before the money was even read; and
    /// a contented starter halved his wage for a smaller club half the
    /// time because nothing in the chain knew what he currently earned.
    ///
    /// Now: one utility, one seeded disposition per negotiation, and a
    /// reservation wage that falls out of the same arithmetic. Wage rounds
    /// move the utility rather than re-rolling the die, so an offer raised
    /// to his demand IS accepted. The old willingness floor survives as a
    /// *consequence* — an important starter with no push has an `S + P − A`
    /// no wage a club can hold will pay for — and its three reasons
    /// survive as [`TermsRefusalCause`] labels.
    fn resolve_personal_terms(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        round: u8,
        date: NaiveDate,
        outcomes: &mut NegotiationOutcomes,
    ) {
        let cfg = AppraisalConfig::default();
        let is_foreign = neg_data.selling_country_id.is_some();

        // ── The two inputs ──────────────────────────────────────
        //
        // The stance is FROZEN for the life of the personal-terms phase,
        // on both paths: built once, on the first round, and staged on the
        // negotiation the way a cross-border deal's has always been.
        //
        // Rebuilding it live at every round was the "raise to the
        // reservation ⇒ deterministic yes" guarantee's one hole. A
        // matchday between rounds moves `starter_ratio`, an `Unh` can
        // clear (−0.35), resignation moves — and because the buyer raises
        // to exactly `1.02 × reservation`, a drift of more than
        // `w_m · ln(1.02)` ≈ 0.01 turned a paid-for yes into a terminal
        // refusal. Foreign deals were already immune.
        //
        // `deadline_urgency` stays live: it sits on the OFFER, it is the
        // buyer's clock rather than the player's situation, and it only
        // ever pushes up.
        //
        // The staged plausibility inputs are the expensive part and both
        // halves want them — importance for the stance, the sporting drop
        // for the offer — so they are built once, together.
        let mut stance = neg_data.staged_stance;
        let mut sporting_drop = neg_data.staged_sporting_drop;
        if stance.is_none() && !is_foreign {
            if let Some((built, drop)) = Self::stance_for(country, neg_data, date) {
                stance = Some(built);
                sporting_drop = Some(drop);
                if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                    negotiation.staged_stance = Some(built);
                    negotiation.staged_sporting_drop = Some(drop);
                }
            }
        }
        // A global-pool free agent has no club, no depth chart and no
        // country to be read from — and he is exactly the man for whom the
        // decision genuinely IS the money against a fair anchor plus the
        // shirt on offer. Everything else stays at its no-view default.
        let stance = stance.unwrap_or_else(|| {
            let offered = neg_data.offered_annual_wage.unwrap_or(0) as f64;
            let fair = neg_data
                .staged_reservation_wage
                .map(|w| w as f64)
                .filter(|w| *w > 0.0)
                .unwrap_or(offered);
            PlayerStance::from_terms(
                neg_data.player_age,
                neg_data.player_ambition,
                offered.max(fair),
                fair,
            )
        });

        // The buyer's opening figure, stamped on the negotiation now if
        // nothing staged one — see [`Self::ensure_opening_salary`].
        let offered_annual_wage = Self::ensure_opening_salary(country, neg_id, neg_data);
        let offer = Self::offer_view_for(
            country,
            neg_data,
            offered_annual_wage,
            &stance,
            sporting_drop,
            date,
        );

        // ε — his private disposition on THIS negotiation, drawn once and
        // re-derived identically on every round.
        let disposition = PlayerDisposition::for_negotiation(
            country.id,
            neg_id,
            neg_data.player_id,
            neg_data.buying_club_id,
            cfg.disposition_sigma,
        );
        let appraisal = PlayerOfferAppraisal::appraise(&stance, &offer, disposition, &cfg);

        if TransferTrace::is(neg_data.player_id) {
            TransferTrace::line(
                neg_data.player_id,
                "terms",
                format!(
                    "round={round} buyer={} age={} ambition={:.2} offered={:.0} anchor={:.0} \
                     {}{} -> {}",
                    neg_data.buying_club_id,
                    neg_data.player_age,
                    neg_data.player_ambition,
                    offer.offered_wage,
                    PlayerOfferAppraisal::anchor(&stance),
                    // What the retired hard floor would have said. Kept as
                    // a diagnostic, not a gate: the whole point of the
                    // appraisal is that the same refusal now has a price.
                    // Only cross-border deals ever set it — printing it on
                    // a domestic one is a constant `false`.
                    if is_foreign {
                        format!("legacy_floor={} ", neg_data.foreign_terms_floor_blocked)
                    } else {
                        String::new()
                    },
                    appraisal.explain(),
                    if appraisal.accepts() { "AGREED" } else { "no" },
                ),
            );
        }

        if appraisal.accepts() {
            if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
                // One number, both slots. The wage he said yes to IS the
                // wage the contract installs — the package's figure and
                // the staged offer are written together by
                // `open_salary_at` / `raise_offered_salary`, and a path
                // that ever set only one of them would install a salary
                // nobody agreed.
                debug_assert!(
                    negotiation
                        .current_offer
                        .personal_terms
                        .as_ref()
                        .and_then(|t| t.annual_wage)
                        .map(|w| Some(w) == negotiation.offered_salary)
                        .unwrap_or(true),
                    "agreed wage diverged from the staged offer"
                );
                negotiation.advance_to_medical(date);
                negotiation.terms_refusal_cause = None;
                negotiation.terms_reservation_wage = Some(appraisal.reservation_wage);
            }
            // Personal terms agreed — the player has said yes and only
            // the medical stands between him and the move. `Trn`
            // replaces `Bid`: match selection now treats him as a
            // near-sold asset (protected in routine games).
            Self::set_saga_status(country, neg_data, PlayerStatusType::Trn, date);
            // …and the saga says so. Every other rung of this ladder
            // files a beat; without this one the feed followed a move
            // from the first scout report to the fee agreement and then
            // went silent on the day it was actually agreed.
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::TermsAgreed,
                TransferInterestSource::ClubBriefing,
                false,
            );
            return;
        }

        // ── The buyer answers the demand, if it can hold it ─────
        //
        // Not a second roll: the disposition is already drawn, so raising
        // the offer to the reservation is a *deterministic* yes. What
        // decides the deal now is whether this buyer's wage power reaches
        // his number — level wage stretched, room under the board's
        // mandate, or a slice of an owner's yearly subsidy (L4).
        const MAX_WAGE_ROUNDS: u8 = 2;
        // Could this buyer have paid his number at all? The answer is the
        // refusal's cause when it is no — a man who would sign for more
        // than the club can hold refused on the WAGE, however the axes
        // read (L3 step 3).
        let power = Self::wage_power_for(country, neg_id, neg_data, &offer);
        // …on a PERMANENT deal. A loan negotiates no wage at all: the
        // borrower picks up the parent's, `offered_wage = anchor` so
        // `M ≡ 0`, and `wage_power_for` returns `ceiling = level_wage =
        // the anchor`. The reservation is `anchor·exp(−(U+ε)/(0.5 w_m))`,
        // so ANY loan with `U + ε < 0` sits above that ceiling and every
        // loan refusal in the world was labelled `WageDemand` — 97 % of
        // the census's causes, on deals where nobody discussed money.
        let wage_unreachable = !neg_data.is_loan
            && power
                .as_ref()
                .map(|p| !p.can_reach(appraisal.reservation_wage as f64))
                .unwrap_or(false);
        if round < MAX_WAGE_ROUNDS {
            if let Some(power) = power {
                let reservation = appraisal.reservation_wage as f64;
                let target = (reservation * 1.02).min(power.ceiling);
                if power.can_reach(reservation) && target > offer.offered_wage {
                    // Close 60 % of the gap, as before — except on the
                    // last round, which closes it, so a deal the buyer
                    // can genuinely afford is not lost to arithmetic.
                    let close = if round + 1 >= MAX_WAGE_ROUNDS {
                        1.0
                    } else {
                        0.6
                    };
                    let improved =
                        (offer.offered_wage + (target - offer.offered_wage) * close).round() as u32;
                    if improved as f64 > offer.offered_wage {
                        if let Some(negotiation) =
                            country.transfer_market.negotiations.get_mut(&neg_id)
                        {
                            negotiation.raise_offered_salary(improved);
                            negotiation.advance_personal_terms_round(date);
                            negotiation.terms_reservation_wage = Some(appraisal.reservation_wage);
                        }
                        return;
                    }
                }
            }
        }

        Self::reject_personal_terms(
            country,
            neg_id,
            neg_data,
            outcomes,
            Some(appraisal.refusal_cause(wage_unreachable)),
            Some(appraisal.reservation_wage),
        );
    }

    /// The player said no. One exit, so the saga beat, the listing reopen
    /// and the diagnostics can never disagree about which of them ran.
    fn reject_personal_terms(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        outcomes: &mut NegotiationOutcomes,
        cause: Option<TermsRefusalCause>,
        reservation_wage: Option<u32>,
    ) {
        if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
            negotiation.reject_with_reason(NegotiationRejectionReason::PlayerRejectedPersonalTerms);
            negotiation.terms_refusal_cause = cause;
            negotiation.terms_reservation_wage = reservation_wage;
        }
        Self::reopen_listing_for_player(country, neg_data.player_id);
        Self::clear_saga_statuses(country, neg_id, neg_data.player_id);
        // He said no. His own answer, not his club's — the ladder
        // already had a rung for the selling club refusing to sell, and
        // reading both as the same "the move died" beat lost the one
        // fact a reader most wants: whose decision it was.
        Self::notify_player_stage(
            country,
            outcomes,
            neg_data,
            TransferInterestStage::TermsRejected,
            TransferInterestSource::ClubBriefing,
            false,
        );
        // A pool free agent (selling_club_id == 0) who declined the terms gets
        // it counted against his market state — the "player actually rejected
        // a negotiated offer" moment, not the candidate scan.
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

    /// The player's side of a DOMESTIC negotiation, rebuilt live, with the
    /// sporting distance of the move alongside it.
    ///
    /// Both come out of one staged-plausibility build: the importance and
    /// the drop are the same read of the same two clubs, and computing them
    /// separately would walk the seller's squad twice per round.
    ///
    /// `None` when the deal is cross-border, when either club has gone, or
    /// when the player cannot be found in this country — the pool free
    /// agent among them, who is staged instead.
    fn stance_for(
        country: &Country,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) -> Option<(PlayerStance, f32)> {
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
        // The ONE importance formula, from the staged plausibility model —
        // the domestic and foreign paths must not disagree about what a
        // player is to his club (Part VIII, "two importance formulas").
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
        let availability =
            AvailabilityView::read(player, neg_data.is_loan, neg_data.listing_origin);
        let stance = PlayerStanceBuilder::build(&StanceInputs {
            player,
            seller_country: country,
            seller_club: seller,
            buyer_club_id: neg_data.buying_club_id,
            rep_diff: neg_data.buying_rep - neg_data.selling_rep,
            importance: TransferPlausibilityEvaluator::player_importance(&inputs),
            listed_by_club: availability.listed_by_club,
            available: availability.available_soft,
            months_to_tournament: country.months_to_tournament_for(player.nationality_continent_id),
            date,
        });
        Some((
            stance,
            TransferPlausibilityEvaluator::sporting_drop(&inputs),
        ))
    }

    /// What is on the table, this round.
    ///
    /// `staged_drop` is the sporting distance read alongside the stance
    /// and frozen with it, so both halves of the appraisal describe the
    /// same moment for every round of the phase.
    fn offer_view_for(
        country: &Country,
        neg_data: &NegotiationData,
        offered_annual_wage: Option<u32>,
        stance: &PlayerStance,
        staged_drop: Option<f32>,
        date: NaiveDate,
    ) -> OfferView {
        let kind = if neg_data.is_loan {
            OfferKind::Loan
        } else {
            OfferKind::Permanent
        };
        // The wage he would actually draw.
        //
        // A LOAN pays nothing new: the borrower picks up a share of the
        // deal he already has, so the offer sits exactly on his anchor and
        // the money axis is silent (Part VIII, "the loan that pays"). A
        // permanent move draws the figure the buyer has tabled.
        let staged_wage = offered_annual_wage.map(|w| w as f64).filter(|w| *w > 0.0);
        let offered_wage = if neg_data.is_loan {
            PlayerOfferAppraisal::anchor(stance)
        } else {
            staged_wage
                .or(neg_data.staged_reservation_wage.map(|w| w as f64))
                .unwrap_or(stance.current_wage)
        };

        let sporting_drop = staged_drop.unwrap_or(
            // No plausibility inputs to hand (a pool free agent, or a club
            // that has gone): fall back to the club-reputation gap alone,
            // on the same 0.55 weight the full read gives it, rather than
            // pretending the move is lateral.
            0.55 * (neg_data.selling_rep - neg_data.buying_rep),
        );

        let mut offer = OfferViewBuilder::build(
            kind,
            country,
            neg_data.buying_club_id,
            stance,
            offered_wage,
            neg_data
                .personal_terms
                .as_ref()
                .and_then(|t| t.squad_status_promise),
            sporting_drop,
        );

        // End-of-window pressure: players prefer a signed deal over an
        // expired negotiation that drops them back into limbo.
        offer.deadline_urgency = Self::deadline_urgency_for(country, date);
        // The escape route he negotiated for himself. He almost always
        // welcomes the move the clause was written for.
        offer.release_clause_triggered =
            neg_data.selling_country_id.is_none() && Self::clause_triggers_sale(country, neg_data);
        // Being asked back by the club that sold him. Context matters:
        // sold cheaply against the current offer reads as having been
        // undervalued; a much bigger club now, or a man old enough to be
        // pragmatic, takes the sting out.
        if let Some((sold_club_id, sold_fee)) = &neg_data.player_sold_from {
            if *sold_club_id == neg_data.buying_club_id {
                let mut sting: f32 = 1.0;
                if *sold_fee > 0.0 && neg_data.offer_amount > sold_fee * 3.0 {
                    sting += 0.4;
                }
                let rep_diff = neg_data.buying_rep - neg_data.selling_rep;
                if rep_diff > 0.3 {
                    sting -= 0.6;
                } else if rep_diff > 0.15 {
                    sting -= 0.3;
                }
                sting -= 0.4 * stance.career_spent;
                offer.returning_to_seller = sting.clamp(0.2, 1.4);
            }
        }
        offer
    }

    /// What the BUYING club can hold for this shirt. The buyer is always in
    /// this country — the negotiation is resolved by its own pass — so this
    /// works identically on domestic and cross-border deals.
    /// The buyer's opening wage, stamped on the negotiation as BOTH the
    /// offer and the anchor when nothing staged one.
    ///
    /// A negotiation created without a salary used to reach the first
    /// appraisal with the PLAYER's own number in the offer slot — the view
    /// falls back through `staged_reservation_wage` to `current_wage` — so
    /// the buyer's "level" and the player's demand were the same figure,
    /// `can_reach` was trivially true, and the buyer raised to 1.02× of
    /// whatever it happened to be. The level is what the buyer's own
    /// reputation says the promised shirt is worth, and it is stamped
    /// before the round rather than latched after one.
    fn ensure_opening_salary(
        country: &mut Country,
        neg_id: u32,
        neg_data: &NegotiationData,
    ) -> Option<u32> {
        if let Some(wage) = neg_data.offered_annual_wage.filter(|w| *w > 0) {
            return Some(wage);
        }
        if neg_data.is_loan {
            // A loan tables no new wage at all — the borrower picks up a
            // share of the deal he already has, and `wage_power_for`
            // reads that directly. Stamping one here would write a salary
            // into the personal-terms package and install it on the loan
            // contract.
            return None;
        }
        let wage = (Self::buyer_level_wage(country, neg_data)? as f64).max(1.0) as u32;
        if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id) {
            negotiation.open_salary_at(wage);
        }
        Some(wage)
    }

    /// [`BuyerLevelWage`] for the shirt the buyer is promising. `None`
    /// when the player is not reachable from this country — a cross-border
    /// deal, which always stages its wage at creation anyway.
    fn buyer_level_wage(country: &Country, neg_data: &NegotiationData) -> Option<u32> {
        let player = find_player_in_country(country, neg_data.player_id)?;
        let promised = neg_data
            .personal_terms
            .as_ref()
            .and_then(|t| t.squad_status_promise);
        Some(BuyerLevelWage::evaluate(
            player,
            neg_data.player_age,
            neg_data.buying_rep,
            neg_data.buying_league_reputation,
            promised,
        ))
    }

    fn wage_power_for(
        country: &Country,
        neg_id: u32,
        neg_data: &NegotiationData,
        offer: &OfferView,
    ) -> Option<WagePower> {
        let buyer = country
            .clubs
            .iter()
            .find(|c| c.id == neg_data.buying_club_id)?;
        // One lookup: the negotiation being resolved is the one whose id
        // was passed in, and scanning the map by (player, buyer) twice
        // found exactly the same row.
        let negotiation = country.transfer_market.negotiations.get(&neg_id);
        // The buyer's own LEVEL is where it opened, not where the last
        // round left it: its power is a stretch on that figure, and
        // re-anchoring on the raised offer would let the ceiling chase the
        // offer up its own ladder.
        let level_wage = negotiation
            .and_then(|n| n.opening_salary)
            .map(|w| w as f64)
            .unwrap_or(offer.offered_wage)
            .max(1.0);
        // A loan draws the parent's wage, not a fresh salary: an owner's
        // subsidy has no business raising one, so a borrower's power is
        // exactly what it already offered.
        if neg_data.is_loan {
            return Some(WagePower {
                level_wage,
                ceiling: level_wage,
                owner_subsidy: 0.0,
                wage_headroom: 0.0,
            });
        }
        // An unbriefed approach is COVER, not an upgrade. Defaulting to B
        // handed the second-largest envelope to every deal that had never
        // been through the planner.
        let tier = negotiation
            .and_then(|n| n.brief_tier)
            .unwrap_or(BriefTier::C);
        let reserved =
            OwnerEnvelopeReservations::open(country, neg_data.buying_club_id, tier, neg_id);
        Some(WagePower::for_player(buyer, level_wage, tier, reserved))
    }

    /// What a shirt about to be signed would draw from its tier envelope —
    /// read off the negotiation BEFORE `complete_transfer` retires the row
    /// that carries the tier and the opening wage.
    ///
    /// Read and apply are two calls because the completion can fail: a
    /// stale row makes `complete_transfer` return `None`, and the draw
    /// used to have been taken already and was never given back. A move
    /// that did not happen must not spend the owner's money.
    fn owner_envelope_draw(country: &Country, neg_id: u32) -> Option<(BriefTier, f64)> {
        let negotiation = country.transfer_market.negotiations.get(&neg_id)?;
        if negotiation.is_loan {
            return None;
        }
        let tier = negotiation.brief_tier.unwrap_or(BriefTier::C);
        let level = negotiation
            .opening_salary
            .or(negotiation.offered_salary)
            .unwrap_or(0) as f64;
        let agreed = negotiation
            .current_offer
            .personal_terms
            .as_ref()
            .and_then(|t| t.annual_wage)
            .or(negotiation.offered_salary)
            .unwrap_or(0) as f64;
        let draw = WagePower::envelope_draw(level, agreed);
        (draw > 0.0).then_some((tier, draw))
    }

    /// A signed shirt draws the part of its wage the buyer's own level
    /// cannot explain from its tier envelope, for the rest of the season.
    ///
    /// Called once, on a completion that actually happened — a negotiation
    /// still in flight is covered by [`OwnerEnvelopeReservations`], and one
    /// that dies gives its share back by simply disappearing from that
    /// fold.
    fn consume_owner_envelope(
        country: &mut Country,
        buying_club_id: u32,
        draw: Option<(BriefTier, f64)>,
    ) {
        let Some((tier, amount)) = draw else {
            return;
        };
        if let Some(buyer) = country.clubs.iter_mut().find(|c| c.id == buying_club_id) {
            if let Some(targets) = buyer.board.season_targets.as_mut() {
                targets.owner_envelopes.consume(tier, amount);
            }
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
            // An agreed move refused at the registration desk — the
            // player had said yes; the collapse is a real story beat.
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::MoveCollapsed,
                TransferInterestSource::ClubBriefing,
                false,
            );
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
                // Snapshot losing bidders BEFORE the cancel flips them to
                // Rejected — each must release its plan's negotiation
                // slot, or the loser sits frozen at its concurrency cap
                // for the rest of the window.
                let losing_bidders: Vec<u32> = country
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
                    .map(|n| n.buying_club_id)
                    .collect();
                country
                    .transfer_market
                    .cancel_negotiations_for_player(neg_data.player_id, neg_id);
                PipelineProcessor::on_negotiation_resolved(
                    country,
                    neg_data.buying_club_id,
                    neg_data.player_id,
                    true,
                );
                for loser in losing_bidders {
                    PipelineProcessor::on_negotiation_resolved(
                        country,
                        loser,
                        neg_data.player_id,
                        false,
                    );
                }
                PipelineProcessor::clear_player_interest(country, neg_data.player_id);
                return;
            }

            // Registration must land inside the window. Negotiations
            // legally survive the close (deals agreed near the deadline
            // finish their paperwork), but a PERMANENT move whose medical
            // resolves after deadline day cannot be registered — it
            // collapses here exactly like a failed medical, instead of
            // registering weeks past the deadline. Loans and free
            // transfers (out-of-contract players) keep their
            // window-independent paths.
            if !neg_data.is_loan && !country.transfer_market.transfer_window_open {
                let signs_free_agent = country
                    .transfer_market
                    .negotiations
                    .get(&neg_id)
                    .and_then(|n| country.transfer_market.listings.get(n.listing_id as usize))
                    .map(|l| l.listing_type == TransferListingType::EndOfContract)
                    .unwrap_or(false);
                if !signs_free_agent {
                    if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id)
                    {
                        negotiation.reject_with_reason(NegotiationRejectionReason::WindowClosed);
                    }
                    Self::reopen_listing_for_player(country, neg_data.player_id);
                    // A fully-agreed deal died at the deadline — the
                    // classic deadline-day heartbreak beat.
                    Self::notify_player_stage(
                        country,
                        outcomes,
                        neg_data,
                        TransferInterestStage::MoveCollapsed,
                        TransferInterestSource::ClubBriefing,
                        false,
                    );
                    Self::clear_saga_statuses(country, neg_id, neg_data.player_id);
                    PipelineProcessor::on_negotiation_resolved(
                        country,
                        neg_data.buying_club_id,
                        neg_data.player_id,
                        false,
                    );
                    return;
                }
            }

            // Reserve the agreed fee against the buyer's transfer budget the
            // moment the deal closes, so two deals agreed in the same window
            // can't both bank on the same money and one then silently
            // collapse when its deferred execution finds the budget already
            // spent. If the budget can't cover it the club genuinely can't
            // fund the move — refuse it cleanly here rather than agree a deal
            // that would evaporate at execution (the old silent drop: market
            // history written then retracted, interest cleared, player never
            // moved). The reservation is released at execution-start, where
            // the real purchase accounting runs. Loans (lighter accounting)
            // and pool free agents (already returned above) are exempt.
            //
            // Reserve only the UPFRONT cash the buyer commits now: installment
            // tranches are paid over time from the balance by the settlement
            // walk, and execution itself already gates on the upfront portion,
            // so reserving the full headline here blocked structured deals the
            // club could genuinely fund — installments couldn't stretch a tight
            // budget at all. Cross-country deals settle upfront, so their
            // upfront IS the full fee.
            if !neg_data.is_loan {
                let upfront = country
                    .transfer_market
                    .negotiations
                    .get(&neg_id)
                    .map(|n| {
                        TransferExecution::upfront_after_installments(
                            neg_data.offer_amount,
                            neg_data.selling_country_id.is_some(),
                            &n.current_offer.clauses,
                        )
                    })
                    .unwrap_or(neg_data.offer_amount);
                let reserved = country
                    .clubs
                    .iter_mut()
                    .find(|c| c.id == neg_data.buying_club_id)
                    .map(|c| {
                        let ok = c.finance.reserve_transfer_budget(upfront);
                        // Mirror the finance reservation on the plan's own
                        // ledger so `available_budget()` reflects committed
                        // deals — these fields were dead (always zero), so
                        // staff-recommendation allocations and broadcast
                        // affordability reads saw the full window pot all
                        // window long.
                        if ok {
                            c.transfer_plan.reserved += upfront;
                        }
                        ok
                    })
                    .unwrap_or(true);
                if !reserved {
                    if let Some(negotiation) = country.transfer_market.negotiations.get_mut(&neg_id)
                    {
                        negotiation
                            .reject_with_reason(NegotiationRejectionReason::AskingPriceTooHigh);
                    }
                    Self::reopen_listing_for_player(country, neg_data.player_id);
                    // The buyer's money wasn't there at the finish line —
                    // an agreed move collapsing on funds.
                    Self::notify_player_stage(
                        country,
                        outcomes,
                        neg_data,
                        TransferInterestStage::MoveCollapsed,
                        TransferInterestSource::ClubBriefing,
                        false,
                    );
                    Self::clear_saga_statuses(country, neg_id, neg_data.player_id);
                    PipelineProcessor::on_negotiation_resolved(
                        country,
                        neg_data.buying_club_id,
                        neg_data.player_id,
                        false,
                    );
                    return;
                }
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

            // Snapshot losing bidders BEFORE `complete_transfer` flips
            // their negotiations to Rejected — each must release its
            // plan's negotiation slot (and advance its shortlist), or the
            // loser sits frozen at its concurrency cap for the rest of
            // the window.
            let losing_bidders: Vec<u32> = country
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
                .map(|n| n.buying_club_id)
                .collect();

            // What the owner's cheque WOULD cover, read while the row
            // still exists — the tier and the opening wage live on the
            // negotiation `complete_transfer` is about to retire. It is
            // only drawn if the completion actually happens.
            let envelope_draw = Self::owner_envelope_draw(country, neg_id);

            if let Some(completed) = country.transfer_market.complete_transfer(
                neg_id,
                date,
                player_name,
                from_team_name,
                to_team_name,
            ) {
                Self::consume_owner_envelope(country, neg_data.buying_club_id, envelope_draw);
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
                for loser in losing_bidders {
                    PipelineProcessor::on_negotiation_resolved(
                        country,
                        loser,
                        neg_data.player_id,
                        false,
                    );
                }
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
            Self::notify_player_stage(
                country,
                outcomes,
                neg_data,
                TransferInterestStage::MoveCollapsed,
                TransferInterestSource::ConfirmedApproach,
                false,
            );
            Self::clear_saga_statuses(country, neg_id, neg_data.player_id);
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
                // A synthetic row existed only to back the bid that just
                // collapsed. Reopening it as Available advertised an
                // unlisted player to the whole market forever (the parent
                // never listed him, and listings are never removed) — it
                // dies with its negotiation instead. Genuine seller
                // listings reopen so the listed pipeline re-engages.
                listing.status = if listing.is_seller_advertised() {
                    TransferListingStatus::Available
                } else {
                    TransferListingStatus::Cancelled
                };
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

    /// Age in days of the seller's own genuine permanent listing for this
    /// negotiation's player — `None` when the club never advertised him.
    /// Reads `Available` and `InNegotiation` rows alike: the clock is the
    /// listing, not the bid currently riding on it.
    fn seller_listing_age_days(
        country: &Country,
        neg_data: &NegotiationData,
        date: NaiveDate,
    ) -> Option<i64> {
        country
            .transfer_market
            .listings
            .iter()
            .filter(|l| {
                l.player_id == neg_data.player_id
                    && l.club_id == neg_data.selling_club_id
                    && l.listing_type == TransferListingType::Transfer
                    && l.origin == TransferListingOrigin::SellerListed
                    && matches!(
                        l.status,
                        TransferListingStatus::Available | TransferListingStatus::InNegotiation
                    )
            })
            .map(|l| (date - l.listed_date).num_days().max(0))
            .max()
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
    /// Contract months at or below which the seller has effectively lost
    /// his leverage: whatever he does not get now he gets nothing for.
    const STRONG_DISTRESS_MONTHS: i64 = 6;
    /// Contract months at or below which a club starts pricing to sell —
    /// the last full window in which a fee is still realistic.
    const MODEST_DISTRESS_MONTHS: i64 = 12;
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
        let base_fraction = Self::floor_fraction(asset_class, distress)?;
        // A genuine listing the club can't shift erodes the floor over time —
        // a seller who's held a player on the market for months is genuinely
        // more willing to deal. Without this a rated, long-listed player's
        // floor stayed permanently above any (decayed) bid and he could only
        // ever leave via the 365-day free exit.
        let fraction = Self::erode_for_seller_position(
            base_fraction,
            country,
            player,
            seller,
            neg_data.player_id,
            date,
        );

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

    /// Days a genuine permanent listing persists before the floor fully eases
    /// to its residual — roughly half a season of fruitless listing.
    const FLOOR_EROSION_DAYS: f64 = 180.0;

    /// A player a club has listed for permanent transfer but can't shift
    /// signals a seller increasingly willing to deal: the floor eases from its
    /// base fraction toward `DISTRESSED_RESIDUAL` across ~half a season of
    /// listing, and never below it (a valued asset is never handed over for
    /// pennies). Only a genuine `SellerListed` listing erodes — synthetic
    /// (unsolicited-approach), loan-out and end-of-contract origins, and
    /// unlisted players, hold firm, so a throwaway listing behind an
    /// unsolicited bid can't game the floor down.
    /// Share of the club's matches at or above which a player is being
    /// used enough that the club has no reason to discount him. Below it
    /// the willingness to deal rises continuously to full by zero minutes.
    const INVOLVEMENT_EROSION_CEILING: f64 = 0.5;

    /// How far the floor has eased, from the strongest of the seller's two
    /// reasons to deal: a listing the market has ignored, or a player the
    /// club has stopped picking.
    ///
    /// Listing age alone left the most common stuck profile untouched. A
    /// well-rated squad player who is never listed — because the surplus
    /// sweeps protect his class — kept his full importance premium
    /// permanently, so no realistic bid could ever clear his floor and the
    /// only exit he had was running his contract down. But a club that has
    /// a player it does not play is exactly a club that will take less for
    /// him, listed or not; that is how squad players actually move. Both
    /// terms ease toward the same residual, so a valued asset is still
    /// never handed over for pennies.
    fn erode_for_seller_position(
        fraction: f64,
        country: &Country,
        player: &Player,
        seller: &Club,
        player_id: u32,
        date: NaiveDate,
    ) -> f64 {
        if fraction <= Self::DISTRESSED_RESIDUAL {
            return fraction;
        }
        let listing_t = Self::listing_age_erosion(country, player_id, date);
        let involvement_t = Self::involvement_erosion(player, seller, date);
        let t = listing_t.max(involvement_t).clamp(0.0, 1.0);
        if t <= 0.0 {
            return fraction;
        }
        fraction - (fraction - Self::DISTRESSED_RESIDUAL) * t
    }

    /// Only a genuine `SellerListed` listing ages the floor down — synthetic
    /// (unsolicited-approach), loan-out and end-of-contract origins hold
    /// firm, so a throwaway listing behind an unsolicited bid can't game the
    /// floor.
    fn listing_age_erosion(country: &Country, player_id: u32, date: NaiveDate) -> f64 {
        let days_listed = country
            .transfer_market
            .get_listing_by_player(player_id)
            .filter(|l| l.origin == TransferListingOrigin::SellerListed)
            .map(|l| (date - l.listed_date).num_days().max(0))
            .unwrap_or(0);
        if days_listed <= 0 {
            return 0.0;
        }
        (days_listed as f64 / Self::FLOOR_EROSION_DAYS).clamp(0.0, 1.0)
    }

    /// Continuous in the share of the club's matches the player has
    /// featured in: none of them eases the floor fully, half of them not at
    /// all. Silent until the season has produced a readable sample, and
    /// silent for players who were unavailable rather than unwanted — a
    /// long-term injury is not a reason to discount someone.
    fn involvement_erosion(player: &Player, seller: &Club, date: NaiveDate) -> f64 {
        let evidence = SquadEvidenceContext::current_season_sample(date, seller);
        if evidence.is_early_season() {
            return 0.0;
        }
        if player.player_attributes.is_injured
            || player.player_attributes.is_banned
            || player.player_attributes.is_in_recovery()
        {
            return 0.0;
        }
        let club_matches = evidence.club_matches_proxy() as f64;
        if club_matches <= 0.0 {
            return 0.0;
        }
        let appearances = (player.statistics.played
            + player.statistics.played_subs
            + player.cup_statistics.played
            + player.cup_statistics.played_subs) as f64;
        let involvement = (appearances / club_matches).clamp(0.0, 1.0);
        ((Self::INVOLVEMENT_EROSION_CEILING - involvement) / Self::INVOLVEMENT_EROSION_CEILING)
            .clamp(0.0, 1.0)
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
        // ── Strong: the player should/can leave below value. ──
        if player.statuses.has(PlayerStatusType::Req) {
            return SellerDistress::Strong;
        }
        if let Some(contract) = player.contract.as_ref() {
            if matches!(contract.squad_status, PlayerSquadStatus::NotNeeded) {
                return SellerDistress::Strong;
            }
            let months_remaining = (contract.expiration - date).num_days() / 30;
            if months_remaining <= Self::STRONG_DISTRESS_MONTHS {
                return SellerDistress::Strong;
            }
            // The year before that is where selling clubs actually start
            // moving: past the final summer window a club either renews or
            // watches him leave for nothing, and it prices him accordingly.
            // Jumping straight from full price at seven months to a waived
            // premium at six left no window in which an expiring contract
            // was cheap enough to move but still worth a fee — so those
            // players ran their deals down instead of being sold, which is
            // the single most avoidable way a squad loses value.
            if months_remaining <= Self::MODEST_DISTRESS_MONTHS {
                return SellerDistress::Modest;
            }
        }
        if seller.finance.balance.balance < 0 {
            return SellerDistress::Strong;
        }
        if Self::unhappiness_is_durable(player) {
            return SellerDistress::Strong;
        }

        // ── Modest: market-leverage signals (available, but not forced). ──
        if player.statuses.has(PlayerStatusType::Lst) || player.statuses.has(PlayerStatusType::Unh)
        {
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
    use crate::transfers::pipeline::{LoanDestinationPreference, LoanOutCandidate, LoanOutStatus};
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
                            preferred_destination: LoanDestinationPreference::Any,
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
                    split_season: false,
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
                staged_reservation_wage: None,
                staged_stance: None,
                staged_sporting_drop: None,
                buying_league_reputation: 5000,
                selling_league_reputation: 5_000,
                player_stage_inclination: 0.0,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms: None,
                foreign_terms_floor_blocked: false,
                foreign_seller_importance: None,
                foreign_seller_finances: None,
            }
        }
    }

    /// Without the owner-staged development candidate, the protected
    /// fresh signing refuses every approach — including loans.
    #[test]
    fn protected_signing_without_dev_listing_rejects_loan_approach() {
        let mut country = ProtectionFixtures::world(false);
        let neg_data = ProtectionFixtures::neg_data(&country);

        let mut outcomes = NegotiationOutcomes {
            deferred: Vec::new(),
            free_agent_signings: Vec::new(),
            free_agent_rejected_ids: Vec::new(),
            player_signals: Vec::new(),
        };
        CountryResult::resolve_initial_approach(
            &mut country,
            ProtectionFixtures::NEG_ID,
            &neg_data,
            ProtectionFixtures::date(),
            &mut outcomes,
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

        let mut outcomes = NegotiationOutcomes {
            deferred: Vec::new(),
            free_agent_signings: Vec::new(),
            free_agent_rejected_ids: Vec::new(),
            player_signals: Vec::new(),
        };
        CountryResult::resolve_initial_approach(
            &mut country,
            ProtectionFixtures::NEG_ID,
            &neg_data,
            ProtectionFixtures::date(),
            &mut outcomes,
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
        /// status / contract expiry. Visible ability is set from `ca` via
        /// skills so the squad-asset classifier (which ranks by observable
        /// level, not the hidden CA) reads the intended pecking order.
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
                .skills(PlayerSkills::flat_for_ability(ca))
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
                    split_season: false,
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
                staged_reservation_wage: None,
                staged_stance: None,
                staged_sporting_drop: None,
                buying_league_reputation: 6000,
                selling_league_reputation: 5_000,
                player_stage_inclination: 0.0,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms: None,
                foreign_terms_floor_blocked: false,
                foreign_seller_importance: None,
                foreign_seller_finances: None,
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
        let target = Ff::player(
            100,
            150,
            26,
            5000,
            PlayerSquadStatus::KeyPlayer,
            Ff::far_contract(),
        );
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

    /// Shared seller squad for the end-to-end floor tests: a recent regular
    /// tagged `NotYetSet` (so the plausibility importance gate does not
    /// fire and only the market-value floor is in play) plus two stronger
    /// squadmates.
    impl Ff {
        fn floor_squad() -> Vec<Player> {
            let mut target = Ff::player(
                100,
                130,
                26,
                2000,
                PlayerSquadStatus::NotYetSet,
                Ff::far_contract(),
            );
            target
                .statistics_history
                .items
                .push(Ff::history_row(2025, 14)); // recent regular
            target.statistics.played = 2; // thin current sample
            let better_a = Ff::player(
                101,
                142,
                27,
                2000,
                PlayerSquadStatus::NotYetSet,
                Ff::far_contract(),
            );
            let better_b = Ff::player(
                102,
                138,
                25,
                2000,
                PlayerSquadStatus::NotYetSet,
                Ff::far_contract(),
            );
            vec![target, better_a, better_b]
        }

        fn insert_negotiation(country: &mut Country, offer_amount: f64) {
            let offer = TransferOffer::new(
                CurrencyValue::new(offer_amount, Currency::Usd),
                Ff::BUYER_ID,
                Ff::date(),
            );
            let neg = TransferNegotiation::new(
                Ff::NEG_ID,
                100,
                0,
                Ff::SELLER_ID,
                Ff::BUYER_ID,
                offer,
                Ff::date(),
                0.75,
                0.40,
                26,
                0.5,
            );
            country.transfer_market.negotiations.insert(Ff::NEG_ID, neg);
        }
    }

    /// Regression #2a: a below-floor bid mid-rounds is NOT an instant
    /// walk-away — the seller stays at the table and the buyer's escalation
    /// is steered up toward the floor, deterministically (a below-floor bid
    /// can never be accepted, so the escalation branch always runs).
    #[test]
    fn below_floor_bid_escalates_toward_floor_instead_of_dying() {
        let mut country = Ff::country(Ff::floor_squad());
        Ff::insert_negotiation(&mut country, 340_000.0);

        // Synthetic-style asking (offer × 1.2) — the exact laundering that
        // made the bug's ratio look healthy. The floor ignores it.
        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);
        let mut outcomes = NegotiationOutcomes {
            deferred: Vec::new(),
            free_agent_signings: Vec::new(),
            free_agent_rejected_ids: Vec::new(),
            player_signals: Vec::new(),
        };
        CountryResult::resolve_club_negotiation(
            &mut country,
            Ff::NEG_ID,
            &nd,
            1,
            Ff::date(),
            &mut outcomes,
        );

        let neg = &country.transfer_market.negotiations[&Ff::NEG_ID];
        assert_eq!(
            neg.status,
            NegotiationStatus::Countered,
            "round 1 must counter (escalate), not terminally reject"
        );
        assert!(
            neg.current_offer.base_fee.amount > 340_000.0,
            "the escalated bid ({}) must move up toward the floor",
            neg.current_offer.base_fee.amount
        );
    }

    /// Regression #2b: end-to-end — a bid still below the market-value
    /// floor on the FINAL round is rejected deterministically (before any
    /// acceptance roll), so no deal can ever CLOSE below the floor. The
    /// low-budget 340K laundered-asking protection is intact, one phase
    /// after the approach.
    #[test]
    fn below_floor_bid_rejected_at_final_round_end_to_end() {
        let mut country = Ff::country(Ff::floor_squad());
        Ff::insert_negotiation(&mut country, 340_000.0);

        let nd = Ff::neg_data(100, 340_000.0, 408_000.0);
        let mut outcomes = NegotiationOutcomes {
            deferred: Vec::new(),
            free_agent_signings: Vec::new(),
            free_agent_rejected_ids: Vec::new(),
            player_signals: Vec::new(),
        };
        CountryResult::resolve_club_negotiation(
            &mut country,
            Ff::NEG_ID,
            &nd,
            3,
            Ff::date(),
            &mut outcomes,
        );

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

    /// P0a regression: a FAIR (at-asking) bid for an important, non-listed
    /// core player must no longer be walled off up front as
    /// `PlayerTooImportant`. Before the floor/ceiling reconciliation the
    /// importance multiplier (~1.85× asking for a core man) was an
    /// unreachable wall, so any at-asking bid died deterministically with
    /// `PlayerTooImportant` before a single acceptance roll — no buyer
    /// could ever sign an established player. The bid may still fail the
    /// acceptance roll (`SellerRefusedToNegotiate`), but never on the wall.
    #[test]
    fn fair_bid_for_core_player_is_not_walled_off_as_too_important() {
        let target = Ff::player(
            100,
            150,
            26,
            5000,
            PlayerSquadStatus::KeyPlayer,
            Ff::far_contract(),
        );
        let mut country = Ff::country(vec![target]);
        // Offer == asking (ratio 1.0), both far above the market-value
        // floor so `SellerFeeFloor` passes and only the (now-removed)
        // importance wall could have rejected it.
        let neg = TransferNegotiation::new(
            Ff::NEG_ID,
            100,
            0,
            Ff::SELLER_ID,
            Ff::BUYER_ID,
            TransferOffer::new(
                CurrencyValue::new(50_000_000.0, Currency::Usd),
                Ff::BUYER_ID,
                Ff::date(),
            ),
            Ff::date(),
            0.75,
            0.40,
            26,
            0.5,
        );
        country.transfer_market.negotiations.insert(Ff::NEG_ID, neg);
        let nd = Ff::neg_data(100, 50_000_000.0, 50_000_000.0);
        let mut outcomes = NegotiationOutcomes {
            deferred: Vec::new(),
            free_agent_signings: Vec::new(),
            free_agent_rejected_ids: Vec::new(),
            player_signals: Vec::new(),
        };
        CountryResult::resolve_initial_approach(
            &mut country,
            Ff::NEG_ID,
            &nd,
            Ff::date(),
            &mut outcomes,
        );

        let neg = &country.transfer_market.negotiations[&Ff::NEG_ID];
        assert_ne!(
            neg.rejection_reason,
            Some(NegotiationRejectionReason::PlayerTooImportant),
            "a fair at-asking bid must not be walled off as too-important"
        );
    }

    /// Regression #3: an explicit `NotNeeded` surplus tag is a strong
    /// distressed reason — the importance premium is waived — but a valuable
    /// player still keeps the low distressed residual floor, so a 340K bid is
    /// rejected (the seller would release him free before giving him away).
    #[test]
    fn not_needed_player_keeps_distressed_residual_floor() {
        let target = Ff::player(
            100,
            150,
            26,
            5000,
            PlayerSquadStatus::NotNeeded,
            Ff::far_contract(),
        );
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
        let mut target = Ff::player(
            100,
            150,
            26,
            5000,
            PlayerSquadStatus::FirstTeamRegular,
            Ff::far_contract(),
        );
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
            .map(|id| {
                Ff::player(
                    id,
                    140,
                    26,
                    1000,
                    PlayerSquadStatus::NotYetSet,
                    Ff::far_contract(),
                )
            })
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
        let target = Ff::player(
            100,
            150,
            26,
            5000,
            PlayerSquadStatus::KeyPlayer,
            Ff::far_contract(),
        );
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

#[cfg(test)]
mod saga_visibility_tests {
    //! The formerly-silent middle of the transfer saga: fee agreed →
    //! `TransferTalksExpected` + `Bid`, personal terms agreed → `Trn`,
    //! any death of the deal surrenders the badges, and a foreign
    //! player's beat rides up on `outcomes.player_signals` instead of
    //! being dropped.

    use super::*;
    use crate::academy::ClubAcademy;
    use crate::club::player::core::builder::PlayerBuilder;
    use crate::league::{DayMonthPeriod, League, LeagueCollection, LeagueSettings};
    use crate::shared::fullname::FullName;
    use crate::shared::{Currency, CurrencyValue, Location};
    use crate::transfers::negotiation::TransferNegotiation;
    use crate::transfers::offer::TransferOffer;
    use crate::{
        Club, ClubColors, ClubFacilities, ClubFinances, ClubStatus, HappinessEventType,
        PersonAttributes, Player, PlayerAttributes, PlayerClubContract, PlayerCollection,
        PlayerPosition, PlayerPositionType, PlayerPositions, PlayerSkills, StaffCollection, Team,
        TeamCollection, TeamReputation, TeamType, TrainingSchedule,
    };
    use chrono::NaiveTime;

    struct Sv;

    impl Sv {
        const SELLER_ID: u32 = 1;
        const BUYER_ID: u32 = 2;
        const PLAYER_ID: u32 = 100;

        fn date() -> NaiveDate {
            NaiveDate::from_ymd_opt(2026, 7, 10).unwrap()
        }

        fn player(id: u32) -> Player {
            let mut attrs = PlayerAttributes::default();
            attrs.current_ability = 120;
            attrs.potential_ability = 120;
            attrs.current_reputation = 2000;
            let mut contract =
                PlayerClubContract::new(50_000, NaiveDate::from_ymd_opt(2029, 6, 30).unwrap());
            contract.squad_status = crate::PlayerSquadStatus::FirstTeamRegular;
            PlayerBuilder::new()
                .id(id)
                .full_name(FullName::new("Saga".to_string(), format!("P{id}")))
                .birth_date(NaiveDate::from_ymd_opt(2000, 1, 1).unwrap())
                .country_id(1)
                .attributes(PersonAttributes::default())
                .skills(PlayerSkills::flat_for_ability(120))
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

        fn club(id: u32, players: Vec<Player>) -> Club {
            let team = Team::builder()
                .id(id * 10)
                .league_id(Some(1))
                .club_id(id)
                .name(format!("club-{id}"))
                .slug(format!("club-{id}"))
                .team_type(TeamType::Main)
                .players(PlayerCollection::new(players))
                .staffs(StaffCollection::new(Vec::new()))
                .reputation(TeamReputation::new(6000, 6000, 6000))
                .training_schedule(TrainingSchedule::new(
                    NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    NaiveTime::from_hms_opt(15, 0, 0).unwrap(),
                ))
                .build()
                .unwrap();
            Club::new(
                id,
                format!("Club {id}"),
                Location::new(1),
                ClubFinances::new(10_000_000, Vec::new()),
                ClubAcademy::new(3),
                ClubStatus::Professional,
                ClubColors::default(),
                TeamCollection::new(vec![team]),
                ClubFacilities::default(),
            )
        }

        fn country(seller_players: Vec<Player>) -> Country {
            let league = League::new(
                1,
                "L".to_string(),
                "l".to_string(),
                1,
                6000,
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
            Country::builder()
                .id(1)
                .code("ru".to_string())
                .slug("russia".to_string())
                .name("Russia".to_string())
                .continent_id(1)
                .leagues(LeagueCollection::new(vec![league]))
                .clubs(vec![
                    Self::club(Self::SELLER_ID, seller_players),
                    Self::club(Self::BUYER_ID, Vec::new()),
                ])
                .build()
                .unwrap()
        }

        fn neg_data(selling_country_id: Option<u32>) -> NegotiationData {
            let offer = TransferOffer::new(
                CurrencyValue::new(1_000_000.0, Currency::Usd),
                2,
                Self::date(),
            );
            let phase = TransferNegotiation::new(
                1,
                Self::PLAYER_ID,
                0,
                1,
                2,
                offer,
                Self::date(),
                0.5,
                0.9,
                26,
                0.5,
            )
            .phase
            .clone();
            NegotiationData {
                player_id: Self::PLAYER_ID,
                selling_club_id: Self::SELLER_ID,
                buying_club_id: Self::BUYER_ID,
                offer_amount: 1_000_000.0,
                is_loan: false,
                has_option_to_buy: false,
                is_unsolicited: true,
                phase,
                selling_rep: 0.50,
                buying_rep: 0.90,
                player_age: 26,
                player_ambition: 0.5,
                asking_price: 1_000_000.0,
                has_market_listing: true,
                player_is_available: true,
                listing_origin: None,
                selling_country_id,
                selling_continent_id: None,
                selling_country_code: String::new(),
                player_sold_from: None,
                player_name: "Saga Target".to_string(),
                selling_club_name: "Club 1".to_string(),
                offered_annual_wage: Some(60_000),
                staged_reservation_wage: None,
                staged_stance: None,
                staged_sporting_drop: None,
                buying_league_reputation: 6000,
                selling_league_reputation: 5_000,
                player_stage_inclination: 0.0,
                sell_on_percentage: None,
                loan_future_fee: None,
                personal_terms: None,
                foreign_terms_floor_blocked: false,
                foreign_seller_importance: None,
                foreign_seller_finances: None,
            }
        }

        fn outcomes() -> NegotiationOutcomes {
            NegotiationOutcomes {
                deferred: Vec::new(),
                free_agent_signings: Vec::new(),
                free_agent_rejected_ids: Vec::new(),
                player_signals: Vec::new(),
            }
        }

        fn target(country: &Country) -> &Player {
            find_player_in_country(country, Self::PLAYER_ID).expect("target present")
        }

        fn has_event(player: &Player, kind: HappinessEventType) -> bool {
            player
                .happiness
                .recent_events
                .iter()
                .any(|e| e.event_type == kind)
        }
    }

    #[test]
    fn fee_agreed_beat_sets_bid_and_fires_talks_expected() {
        let mut country = Sv::country(vec![Sv::player(Sv::PLAYER_ID)]);
        let mut outcomes = Sv::outcomes();
        let nd = Sv::neg_data(None);

        CountryResult::notify_player_stage(
            &mut country,
            &mut outcomes,
            &nd,
            TransferInterestStage::NegotiationsOpened,
            TransferInterestSource::ClubBriefing,
            false,
        );
        CountryResult::set_saga_status(&mut country, &nd, PlayerStatusType::Bid, Sv::date());

        let p = Sv::target(&country);
        assert!(
            Sv::has_event(p, HappinessEventType::TransferTalksExpected),
            "the fee-agreed beat must surface as TransferTalksExpected"
        );
        assert!(
            p.statuses.has(PlayerStatusType::Bid),
            "an accepted bid must stamp the Bid badge"
        );
        assert!(
            outcomes.player_signals.is_empty(),
            "domestic beats deliver inline"
        );
    }

    #[test]
    fn terms_agreed_swaps_bid_for_trn() {
        let mut country = Sv::country(vec![Sv::player(Sv::PLAYER_ID)]);
        let nd = Sv::neg_data(None);

        CountryResult::set_saga_status(&mut country, &nd, PlayerStatusType::Bid, Sv::date());
        CountryResult::set_saga_status(&mut country, &nd, PlayerStatusType::Trn, Sv::date());

        let p = Sv::target(&country);
        assert!(
            !p.statuses.has(PlayerStatusType::Bid),
            "Trn replaces Bid — the deal has moved past the fee stage"
        );
        assert!(
            p.statuses.has(PlayerStatusType::Trn),
            "agreed personal terms must stamp the Trn badge"
        );
    }

    #[test]
    fn dead_saga_clears_the_badges() {
        let mut country = Sv::country(vec![Sv::player(Sv::PLAYER_ID)]);
        let nd = Sv::neg_data(None);
        CountryResult::set_saga_status(&mut country, &nd, PlayerStatusType::Trn, Sv::date());

        CountryResult::clear_saga_statuses(&mut country, 1, Sv::PLAYER_ID);

        let p = Sv::target(&country);
        assert!(
            !p.statuses.has(PlayerStatusType::Bid) && !p.statuses.has(PlayerStatusType::Trn),
            "a dead deal must surrender both saga badges"
        );
    }

    #[test]
    fn surviving_second_bidder_keeps_the_badge() {
        let mut country = Sv::country(vec![Sv::player(Sv::PLAYER_ID)]);
        let nd = Sv::neg_data(None);
        CountryResult::set_saga_status(&mut country, &nd, PlayerStatusType::Bid, Sv::date());

        // A SECOND buyer's negotiation for the same player is already past
        // the fee-agreed line — the dying first deal must not strip the
        // badge the survivor still justifies.
        let offer = TransferOffer::new(
            CurrencyValue::new(2_000_000.0, Currency::Usd),
            2,
            Sv::date(),
        );
        let mut survivor = TransferNegotiation::new(
            99,
            Sv::PLAYER_ID,
            0,
            Sv::SELLER_ID,
            7,
            offer,
            Sv::date(),
            0.5,
            0.9,
            26,
            0.5,
        );
        survivor.advance_to_personal_terms(Sv::date());
        country.transfer_market.negotiations.insert(99, survivor);

        CountryResult::clear_saga_statuses(&mut country, 1, Sv::PLAYER_ID);

        let p = Sv::target(&country);
        assert!(
            p.statuses.has(PlayerStatusType::Bid),
            "a live second bid past the fee line keeps the badge alive"
        );
    }

    #[test]
    fn foreign_beat_is_deferred_not_dropped() {
        let mut country = Sv::country(Vec::new());
        let mut outcomes = Sv::outcomes();
        let nd = Sv::neg_data(Some(42));

        CountryResult::notify_player_stage(
            &mut country,
            &mut outcomes,
            &nd,
            TransferInterestStage::MoveCollapsed,
            TransferInterestSource::ConfirmedApproach,
            false,
        );

        assert_eq!(
            outcomes.player_signals.len(),
            1,
            "the beat must ride up for Phase C"
        );
        let s = &outcomes.player_signals[0];
        assert_eq!(s.player_id, Sv::PLAYER_ID);
        assert_eq!(s.selling_country_id, 42);
        assert_eq!(s.interested_club_id, Sv::BUYER_ID);
        assert_eq!(s.stage, TransferInterestStage::MoveCollapsed);
        assert_eq!(
            s.buyer_country_id, 1,
            "buyer country is the processing country"
        );
    }

    #[test]
    fn pool_free_agent_gets_no_saga_badges() {
        let mut country = Sv::country(vec![Sv::player(Sv::PLAYER_ID)]);
        let mut nd = Sv::neg_data(None);
        nd.selling_club_id = 0; // global-pool free agent

        CountryResult::set_saga_status(&mut country, &nd, PlayerStatusType::Bid, Sv::date());
        let p = Sv::target(&country);
        assert!(
            !p.statuses.has(PlayerStatusType::Bid),
            "pool free agents have their own market-state machinery"
        );
    }
}

#[cfg(test)]
mod seller_windfall_tests {
    use super::SellerWindfall;

    /// A mid-table Super Lig club: roughly $113M a year of income, a $72M
    /// wage bill and a $72M wage budget — the club that held the reported
    /// player for seven seasons.
    struct Exporter;

    impl Exporter {
        const INCOME: i64 = 113_000_000;
        const WAGES: i64 = 71_900_000;
        const WAGE_BUDGET: i64 = 72_200_000;

        fn windfall(fee: f64) -> SellerWindfall {
            SellerWindfall::of(fee, Self::INCOME, Self::WAGES, Self::WAGE_BUDGET)
        }
    }

    #[test]
    fn an_ordinary_fee_changes_nothing() {
        // A $10M bid is 9% of the year — below `W0`. The seller behaves
        // exactly as it always did, which is what keeps peer-to-peer
        // haggling unchanged.
        let w = Exporter::windfall(10_000_000.0);
        assert_eq!(w.ratio, 0.0, "ratio {}", w.ratio);
    }

    #[test]
    fn a_fee_worth_half_the_year_is_answered_by_the_chairman() {
        // $53M against $113M of income — the reported player's own market
        // value. This is the case the whole model exists for.
        let w = Exporter::windfall(53_000_000.0);
        assert!(
            w.ratio > 0.6,
            "a fee worth ~47% of annual income must read as a near-total windfall, got {}",
            w.ratio
        );
        assert!(w.reservation_ease() > 0.15, "{}", w.reservation_ease());
    }

    #[test]
    fn the_windfall_saturates_rather_than_running_away() {
        let huge = Exporter::windfall(500_000_000.0);
        assert_eq!(huge.ratio, 1.0);
        assert!(
            huge.reservation_ease() <= 0.35,
            "the reservation ease must stay bounded, got {}",
            huge.reservation_ease()
        );
    }

    #[test]
    fn a_club_at_its_wage_ceiling_eases_even_on_a_modest_fee() {
        // Wage bill at 99.6% of budget: the sale funds the squad. The ease
        // is real but small — this is acceptance, never availability.
        let pressed = SellerWindfall::of(10_000_000.0, Exporter::INCOME, 72_000_000, 72_200_000);
        assert_eq!(pressed.ratio, 0.0, "the fee itself is ordinary");
        assert!(pressed.wage_pressure > 0.0, "{}", pressed.wage_pressure);
        assert!(
            pressed.reservation_ease() <= 0.10,
            "wage pressure alone must not move the reservation far: {}",
            pressed.reservation_ease()
        );
    }

    #[test]
    fn a_club_with_wage_room_feels_no_pressure() {
        let roomy = SellerWindfall::of(10_000_000.0, Exporter::INCOME, 40_000_000, 72_200_000);
        assert_eq!(roomy.wage_pressure, 0.0);
    }

    #[test]
    fn an_unknown_seller_behaves_exactly_as_before() {
        // No income on record (a freshly generated world, or a foreign
        // seller whose snapshot is missing): the model must be inert, not
        // guess.
        let unknown = SellerWindfall::of(50_000_000.0, 0, 0, 0);
        assert_eq!(unknown.ratio, 0.0);
        assert_eq!(unknown.reservation_ease(), 0.0);
        assert_eq!(SellerWindfall::none().reservation_ease(), 0.0);
    }
}
