use crate::club::board::{ClubVision, FinancialStance, SigningPreference, VisionYouthFocus};
use crate::club::player::calculators::WageCalculator;
use crate::club::staff::perception::PotentialEstimator;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::offer::{
    PersonalTermsOffer, PromisedSquadStatus, TransferClause, TransferOffer,
};
use crate::transfers::pipeline::{
    BoardRecruitmentDossier, TransferApproach, TransferNeedPriority, TransferNeedReason,
    TransferRequest,
};
use crate::transfers::window::PlayerValuationCalculator;
use crate::utils::FormattingUtils;
use crate::{ClubPhilosophy, Person, Player, PlayerPositionType, PlayerStatusType};
use chrono::{Datelike, NaiveDate};

// ============================================================
// Policies — the four sub-policies that make up a club's
// recruitment identity. Each is a small bag of dials with
// sane defaults; richer constructors derive realistic values
// from board vision + philosophy + squad context.
// ============================================================

/// What kind of players the club wants to bring in and what
/// supporting evidence it requires before pulling the trigger.
#[derive(Debug, Clone)]
pub struct RecruitmentPolicy {
    pub philosophy: ClubPhilosophy,
    pub financial_stance: FinancialStance,
    pub signing_preference: SigningPreference,
    pub youth_focus: VisionYouthFocus,
    pub age_preference: AgePreference,
    /// Minimum scouting confidence to seriously pursue (0..1).
    pub min_scouting_confidence: f32,
    /// 0..1. How strongly the club protects resale value when
    /// structuring signings (sell-ons, contract length on youth).
    pub resale_value_sensitivity: f32,
    /// 0..1. Bias toward home-grown / domestic / value-region targets.
    pub domestic_bias: f32,
}

impl Default for RecruitmentPolicy {
    fn default() -> Self {
        RecruitmentPolicy {
            philosophy: ClubPhilosophy::Balanced,
            financial_stance: FinancialStance::Balanced,
            signing_preference: SigningPreference::Anyone,
            youth_focus: VisionYouthFocus::Balanced,
            age_preference: AgePreference::Balanced,
            min_scouting_confidence: 0.35,
            resale_value_sensitivity: 0.4,
            domestic_bias: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgePreference {
    Youth,
    Prime,
    Veteran,
    Balanced,
}

/// How the club behaves at the table: how hard it pushes on
/// price, how creative it gets with clauses, how exposed it is
/// to wage demands.
#[derive(Debug, Clone)]
pub struct NegotiationPolicy {
    /// 0..1. Drives offer % of anchor and budget cap.
    pub buying_aggressiveness: f32,
    /// 0..1. Higher = less willing to overpay wages.
    pub wage_discipline: f32,
    /// 0..1. Higher = less willing to overpay fees.
    pub fee_discipline: f32,
    /// Multiplier of market value the club will accept as a hard
    /// ceiling. 1.0 = exactly market; 1.6 = will pay up to +60%.
    pub max_overpay_ratio: f32,
    /// 0..1. Preference for installments over upfront cash.
    pub installment_preference: f32,
    /// 0..1. Preference for add-ons over base fee.
    pub addon_preference: f32,
    /// 0..1. How readily the club attaches sell-on clauses.
    pub sell_on_preference: f32,
    /// 0..1. Preference for loans vs permanent.
    pub loan_preference: f32,
    /// 0..1. Tolerance for risky profiles (injury history, attitude).
    pub risk_appetite: f32,
}

impl Default for NegotiationPolicy {
    fn default() -> Self {
        NegotiationPolicy {
            buying_aggressiveness: 0.5,
            wage_discipline: 0.5,
            fee_discipline: 0.5,
            max_overpay_ratio: 1.35,
            installment_preference: 0.4,
            addon_preference: 0.3,
            sell_on_preference: 0.4,
            loan_preference: 0.3,
            risk_appetite: 0.5,
        }
    }
}

/// Where the club sits in its build cycle. Drives urgency
/// and which profiles are attractive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SquadPhase {
    Rebuild,
    TitlePush,
    Survival,
    Consolidation,
    YouthCycle,
}

#[derive(Debug, Clone)]
pub struct SquadBuildingPolicy {
    pub phase: SquadPhase,
    /// 0..1. How aggressively the club acts to plug short-term needs.
    pub short_term_urgency: f32,
}

impl Default for SquadBuildingPolicy {
    fn default() -> Self {
        SquadBuildingPolicy {
            phase: SquadPhase::Consolidation,
            short_term_urgency: 0.5,
        }
    }
}

/// Sell-side policy. Cleanly separated so seller acceptance
/// and asking-price logic can use the same dials a buying
/// strategy exposes for purchases.
#[derive(Debug, Clone)]
pub struct SellingPolicy {
    /// 0..1. Baseline willingness to entertain offers.
    pub willingness_baseline: f32,
    /// 0..1. How much the club resists selling home-grown players.
    pub keep_homegrown_bias: f32,
    /// 0..1. How much cash pressure pushes the club to sell.
    pub cash_pressure: f32,
    /// 0..1. How willing to sell aging players the club is
    /// rotating out of the squad.
    pub sell_aging_bias: f32,
    /// 0..1. How willing to let surplus depth go.
    pub sell_surplus_bias: f32,
    /// 0..1. Resistance to selling to direct rivals.
    pub rival_resistance: f32,
}

impl Default for SellingPolicy {
    fn default() -> Self {
        SellingPolicy {
            willingness_baseline: 0.5,
            keep_homegrown_bias: 0.4,
            cash_pressure: 0.2,
            sell_aging_bias: 0.55,
            sell_surplus_bias: 0.6,
            rival_resistance: 0.7,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SellingDecision {
    Reject,
    Listen,
    Encourage,
}

// ============================================================
// Interest scoring
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestDecision {
    Pursue,
    Consider,
    Pass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestReason {
    PositionNeed,
    RoleFit,
    AgeFitsPolicy,
    HighPotential,
    StrongScoutSupport,
    BoardSupport,
    ExpiringContract,
    PriorityRequest,
    Affordable,
    DomesticBonus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferInterestRisk {
    AgeRisk,
    PoorAttitude,
    WageDemands,
    InjuryConcern,
    LowScoutingConfidence,
    OverBudget,
    OverAgePolicy,
    UnderQuality,
    ContractExpiringRisk,
    RivalSeller,
}

#[derive(Debug, Clone)]
pub struct TransferInterestScore {
    pub score: f32,
    pub decision: TransferInterestDecision,
    pub reasons: Vec<TransferInterestReason>,
    pub risks: Vec<TransferInterestRisk>,
}

// ============================================================
// Strategy context — every input that should bend offer
// construction and interest evaluation, in one struct so the
// signature stays sane.
// ============================================================

#[derive(Debug, Clone)]
pub struct TransferStrategyContext<'a> {
    pub date: NaiveDate,
    pub request: Option<&'a TransferRequest>,
    pub board_dossier: Option<&'a BoardRecruitmentDossier>,
    pub approach: TransferApproach,
    /// 0..1 normalized reputation of the buying club.
    pub buyer_reputation_score: f32,
    /// 0..1 normalized reputation of the selling club.
    pub seller_reputation_score: f32,
    /// Raw league reputation of the buying club (engine 0..10000 scale).
    pub league_reputation: u16,
    /// What this club can spend right now (budget actually allocated).
    pub available_budget: f64,
    /// What this shortlist row has been allocated for the move.
    pub allocated_budget: f64,
    /// Optional wage headroom for the signing.
    pub wage_budget_headroom: Option<f64>,
    /// Current cash balance — drives installment preference.
    pub buying_club_balance: i64,
    pub is_january: bool,
    pub price_level: f32,
    /// Where on the shortlist this candidate sits (0 = first).
    pub shortlist_rank: Option<u8>,
    /// Number of other clubs known to be circling.
    pub competition_count: Option<u8>,
    /// Scout's assessed ability/potential — used in place of
    /// hidden PA wherever scouting context is available.
    pub scout_assessed_ability: Option<u8>,
    pub scout_assessed_potential: Option<u8>,
    pub scout_confidence: Option<f32>,
    pub seller_is_rival: bool,
}

impl<'a> TransferStrategyContext<'a> {
    /// Cheap context for callers that don't have the full
    /// pipeline state on hand (mostly tests and the simple
    /// `calculate_initial_offer` back-compat wrapper).
    pub fn minimal(date: NaiveDate) -> Self {
        TransferStrategyContext {
            date,
            request: None,
            board_dossier: None,
            approach: TransferApproach::PermanentTransfer,
            buyer_reputation_score: 0.5,
            seller_reputation_score: 0.5,
            league_reputation: 5000,
            available_budget: 0.0,
            allocated_budget: 0.0,
            wage_budget_headroom: None,
            buying_club_balance: 0,
            is_january: matches!(date.month(), 1),
            price_level: 1.0,
            shortlist_rank: None,
            competition_count: None,
            scout_assessed_ability: None,
            scout_assessed_potential: None,
            scout_confidence: None,
            seller_is_rival: false,
        }
    }

    /// Whether this approach is any kind of loan (with or without option).
    pub fn is_loan(&self) -> bool {
        !matches!(self.approach, TransferApproach::PermanentTransfer)
    }

    pub fn has_option_to_buy(&self) -> bool {
        matches!(self.approach, TransferApproach::LoanWithOption)
    }
}

// ============================================================
// ClubTransferStrategy — the umbrella struct callers consume.
// Fields are public so call sites can build them inline (the
// negotiations layer still does that) but realistic defaults
// flow through `with_*` helpers and the `from_club_context`
// builder.
// ============================================================

pub struct ClubTransferStrategy {
    pub club_id: u32,
    /// What the strategy is allowed to spend on this signing.
    /// Kept Option<CurrencyValue> for back-compat with the
    /// pre-refactor field name; defaults to "no cap" when None.
    pub budget: Option<CurrencyValue>,
    /// Used as a coarse quality bar for `decide_player_interest`
    /// — typically the average squad current_ability of the
    /// buying club's main team.
    pub reputation_level: u16,
    /// The buying club's real market-value reputation (0..10000),
    /// used ONLY to value targets (`calculate_initial_offer_with_context`).
    /// Kept separate from `reputation_level` because that field carries
    /// squad-average ability, not reputation — feeding an ability value
    /// into the valuation over-priced strong-squad buyers and under-priced
    /// weak-squad ones versus the seller's own market value, which opened a
    /// permanent floor-vs-ceiling gap. Defaults (in `from_club_context`) to
    /// the historical `reputation_level * 100` so untouched callers behave
    /// exactly as before; production wires the real score via
    /// [`Self::with_valuation_reputation`].
    pub valuation_reputation: u16,
    /// Positions actively being recruited. Empty = open.
    pub target_positions: Vec<PlayerPositionType>,

    pub recruitment: RecruitmentPolicy,
    pub negotiation: NegotiationPolicy,
    pub squad_building: SquadBuildingPolicy,
    pub selling: SellingPolicy,
}

impl ClubTransferStrategy {
    /// Minimal constructor. Existing call sites used this and
    /// then mutated fields directly. New code should prefer
    /// `from_club_context`.
    pub fn new(club_id: u32) -> Self {
        ClubTransferStrategy {
            club_id,
            budget: None,
            reputation_level: 50,
            valuation_reputation: 5000,
            target_positions: Vec::new(),
            recruitment: RecruitmentPolicy::default(),
            negotiation: NegotiationPolicy::default(),
            squad_building: SquadBuildingPolicy::default(),
            selling: SellingPolicy::default(),
        }
    }

    /// Builder used by the negotiations pipeline. Derives a
    /// realistic policy bundle from board vision + philosophy
    /// without exposing the caller to every dial.
    pub fn from_club_context(
        club_id: u32,
        budget: Option<CurrencyValue>,
        reputation_level: u16,
        target_positions: Vec<PlayerPositionType>,
        philosophy: &ClubPhilosophy,
        vision: &ClubVision,
        buying_aggressiveness: f32,
    ) -> Self {
        let recruitment = RecruitmentPolicy::from_vision(philosophy, vision);
        let mut negotiation = NegotiationPolicy::from_vision(vision, philosophy);
        negotiation.buying_aggressiveness = buying_aggressiveness.clamp(0.05, 0.95);
        let squad_building = SquadBuildingPolicy::from_vision(vision);
        let selling = SellingPolicy::from_vision(vision, philosophy);

        ClubTransferStrategy {
            club_id,
            budget,
            reputation_level,
            // Preserve the historical valuation basis by default so callers
            // that don't wire a real score (tests, back-compat) behave
            // exactly as before; production overrides via
            // `with_valuation_reputation`.
            valuation_reputation: reputation_level.saturating_mul(100).min(10_000),
            target_positions,
            recruitment,
            negotiation,
            squad_building,
            selling,
        }
    }

    /// Set the real market-value reputation used to value targets. The
    /// builder keeps `from_club_context`'s signature stable while letting
    /// the negotiation pipeline supply the club's actual reputation (rather
    /// than the squad-average-ability fallback baked into the default).
    pub fn with_valuation_reputation(mut self, reputation: u16) -> Self {
        self.valuation_reputation = reputation;
        self
    }

    // ---- Back-compat shims ----------------------------------

    /// Coarse boolean interest used by old code paths. Delegates
    /// to the richer scoring function and turns Pursue/Consider
    /// into `true`.
    pub fn decide_player_interest(&self, player: &Player, date: NaiveDate) -> bool {
        let ctx = TransferStrategyContext::minimal(date);
        match self.evaluate_interest(player, &ctx).decision {
            TransferInterestDecision::Pursue | TransferInterestDecision::Consider => true,
            TransferInterestDecision::Pass => false,
        }
    }

    /// Back-compat entry point. Builds a minimal context and
    /// delegates to the policy-aware variant so behaviour stays
    /// in one place.
    pub fn calculate_initial_offer(
        &self,
        player: &Player,
        asking_price: &CurrencyValue,
        current_date: NaiveDate,
    ) -> TransferOffer {
        let mut ctx = TransferStrategyContext::minimal(current_date);
        ctx.available_budget = self.budget.as_ref().map(|b| b.amount).unwrap_or(0.0);
        ctx.allocated_budget = ctx.available_budget;
        self.calculate_initial_offer_with_context(player, asking_price, &ctx)
    }

    // ---- Richer entry points --------------------------------

    /// Score a player as a recruitment target. Uses assessed
    /// scouting values where present and falls back to public
    /// `current_ability` (never hidden potential_ability) where
    /// not. Caller can read `decision`, `reasons`, `risks` to
    /// drive UI and downstream filters.
    pub fn evaluate_interest(
        &self,
        player: &Player,
        ctx: &TransferStrategyContext,
    ) -> TransferInterestScore {
        let mut score: f32 = 0.0;
        let mut reasons: Vec<TransferInterestReason> = Vec::new();
        let mut risks: Vec<TransferInterestRisk> = Vec::new();

        let age = player.age(ctx.date);

        // Position fit. Empty target list = generic interest.
        let position_open = self.target_positions.is_empty();
        let position_match = position_open || self.target_positions.contains(&player.position());
        if position_match {
            score += 1.0;
            if !position_open {
                reasons.push(TransferInterestReason::PositionNeed);
            }
        } else {
            score -= 1.5;
        }

        // Request priority lifts the floor.
        if let Some(req) = ctx.request {
            match req.priority {
                TransferNeedPriority::Critical => {
                    score += 1.0;
                    reasons.push(TransferInterestReason::PriorityRequest);
                }
                TransferNeedPriority::Important => score += 0.5,
                TransferNeedPriority::Optional => {}
            }
            if (req.preferred_age_min..=req.preferred_age_max).contains(&age) {
                score += 0.4;
                reasons.push(TransferInterestReason::AgeFitsPolicy);
            } else {
                score -= 0.3;
                risks.push(TransferInterestRisk::OverAgePolicy);
            }
        }

        // Age vs club policy.
        match self.recruitment.age_preference {
            AgePreference::Youth if age <= 23 => score += 0.5,
            AgePreference::Prime if (24..=29).contains(&age) => score += 0.5,
            AgePreference::Veteran if age >= 28 => score += 0.3,
            AgePreference::Balanced => {}
            _ => {}
        }
        if matches!(self.recruitment.youth_focus, VisionYouthFocus::DevelopYouth) && age <= 23 {
            score += 0.3;
            reasons.push(TransferInterestReason::AgeFitsPolicy);
        }
        if matches!(self.recruitment.philosophy, ClubPhilosophy::DevelopAndSell) && age >= 30 {
            score -= 0.5;
            risks.push(TransferInterestRisk::AgeRisk);
        }

        // Quality bar. Prefer scout-assessed ability over hidden CA
        // when present — keeps AI honest about what it knows.
        let assessed_ability =
            ctx.scout_assessed_ability
                .unwrap_or(player.player_attributes.current_ability) as u16;
        if assessed_ability < self.reputation_level / 2 {
            score -= 1.0;
            risks.push(TransferInterestRisk::UnderQuality);
        }
        if assessed_ability > self.reputation_level * 2
            && self.negotiation.buying_aggressiveness < 0.8
        {
            // Out-of-tier target — only an aggressive buyer chases.
            score -= 1.0;
        }

        // Potential — use assessed potential whenever scouting context exists.
        if let Some(pot) = ctx.scout_assessed_potential {
            let gap = pot as i16 - assessed_ability as i16;
            if gap >= 15 {
                score += 0.6;
                reasons.push(TransferInterestReason::HighPotential);
            }
        }

        // Scout / board support.
        if let Some(conf) = ctx.scout_confidence {
            if conf < self.recruitment.min_scouting_confidence {
                score -= 0.8;
                risks.push(TransferInterestRisk::LowScoutingConfidence);
            } else if conf > 0.7 {
                score += 0.4;
                reasons.push(TransferInterestReason::StrongScoutSupport);
            }
        }
        if let Some(dossier) = ctx.board_dossier {
            if dossier.chief_scout_support {
                score += 0.4;
                reasons.push(TransferInterestReason::BoardSupport);
            }
            if dossier.avg_confidence < self.recruitment.min_scouting_confidence {
                score -= 0.5;
                risks.push(TransferInterestRisk::LowScoutingConfidence);
            }
            if dossier.risk_flag_count >= 3 && self.negotiation.risk_appetite < 0.4 {
                score -= 0.6;
            }
            score += dossier.consensus_score * 0.3;
            if dossier.budget_fit > 1.3 {
                score -= 0.5;
                risks.push(TransferInterestRisk::OverBudget);
            } else if dossier.budget_fit < 0.8 {
                reasons.push(TransferInterestReason::Affordable);
            }
        }

        // Contract expiry — bargain potential & risk together.
        if let Some(contract) = player.contract.as_ref() {
            let months_remaining = ContractTiming::months_between(ctx.date, contract.expiration);
            if months_remaining <= 6 {
                score += 0.5;
                reasons.push(TransferInterestReason::ExpiringContract);
            } else if months_remaining <= 12 {
                score += 0.2;
                reasons.push(TransferInterestReason::ExpiringContract);
            }
        }

        // Status hints.
        if player.statuses.has(PlayerStatusType::Lst) {
            score += 0.4;
        }
        if player.statuses.has(PlayerStatusType::Req) || player.statuses.has(PlayerStatusType::Unh)
        {
            score += 0.3;
        }
        if player.statuses.has(PlayerStatusType::Inj) {
            risks.push(TransferInterestRisk::InjuryConcern);
            score -= 0.3;
        }

        // Personality concerns are character risk if low and we
        // are risk-averse. Determination is a stable proxy.
        if player.skills.mental.determination < 8.0 {
            risks.push(TransferInterestRisk::PoorAttitude);
            if self.negotiation.risk_appetite < 0.4 {
                score -= 0.4;
            }
        }

        // Rival sellers shave score because the political cost is
        // non-trivial even when the deal otherwise stacks up.
        if ctx.seller_is_rival {
            score -= 0.2;
            risks.push(TransferInterestRisk::RivalSeller);
        }

        // Domestic / signing-preference fit. Without nationality
        // context plumbed in we apply the bias as a small lift
        // when the policy actively prefers domestic — exact
        // matching is the caller's job once nationality is wired.
        if self.recruitment.domestic_bias > 0.5 {
            reasons.push(TransferInterestReason::DomesticBonus);
            score += self.recruitment.domestic_bias * 0.2;
        }

        let decision = if score >= 1.5 {
            TransferInterestDecision::Pursue
        } else if score >= 0.3 {
            TransferInterestDecision::Consider
        } else {
            TransferInterestDecision::Pass
        };

        TransferInterestScore {
            score,
            decision,
            reasons,
            risks,
        }
    }

    /// Policy-aware offer construction. The old method is a
    /// thin wrapper over this with a minimal context.
    pub fn calculate_initial_offer_with_context(
        &self,
        player: &Player,
        asking_price: &CurrencyValue,
        ctx: &TransferStrategyContext,
    ) -> TransferOffer {
        let max_budget = self.budget.as_ref().map(|b| b.amount).unwrap_or(f64::MAX);

        // Valuation anchored on the buying club's real league/club
        // reputation. `valuation_reputation` is the club's market-value
        // score; the league side prefers the live context when present.
        // (Feeding squad-average CA here — the historical bug — made a
        // strong-squad buyer over-value every target and a weak-squad one
        // under-value them versus the seller's own market value, which
        // opened a permanent seller-floor-above-buyer-ceiling gap.)
        let club_rep_for_value = self.valuation_reputation.min(10_000);
        let league_rep_for_value = if ctx.league_reputation > 0 {
            ctx.league_reputation
        } else {
            club_rep_for_value
        };
        let player_value = PlayerValuationCalculator::calculate_value(
            player,
            ctx.date,
            league_rep_for_value,
            club_rep_for_value,
        );

        let aggression = self.negotiation.buying_aggressiveness as f64;
        let fee_discipline = self.negotiation.fee_discipline as f64;

        // For a loan, the incoming `asking_price` is already the loan FEE
        // (a few percent of the player's value), not his permanent price.
        // The permanent anchor below floors the offer at 85% of full value
        // — applying that to a loan re-inflates the fee back to nearly the
        // whole transfer price (the "loan fee == full price" bug). Loans
        // anchor purely on the advertised loan fee instead.
        let is_loan = ctx.is_loan();

        // 1) Anchor: blend asking price and our valuation.
        let mut offer_amount = if is_loan {
            // Negotiate around the loan fee; a free loan (asking 0) stays free.
            asking_price.amount * (0.80 + aggression * 0.18)
        } else if asking_price.amount > 0.0 {
            let market_anchor = asking_price.amount.max(player_value.amount * 0.85);
            market_anchor * (0.74 + aggression * 0.23)
        } else {
            player_value.amount * (0.78 + aggression * 0.18)
        };

        // 2) Adjustments that move the anchor before the cap.

        // Expiring contract → drop the anchor (player is on the
        // way out anyway, seller's leverage is weak).
        if let Some(contract) = player.contract.as_ref() {
            let months_remaining = ContractTiming::months_between(ctx.date, contract.expiration);
            if months_remaining <= 6 {
                offer_amount *= 0.65;
            } else if months_remaining <= 12 {
                offer_amount *= 0.85;
            }
        }

        // Transfer-listed / unhappy player → softer anchor.
        if player.statuses.has(PlayerStatusType::Lst) {
            offer_amount *= 0.9;
        }
        if player.statuses.has(PlayerStatusType::Req) || player.statuses.has(PlayerStatusType::Unh)
        {
            offer_amount *= 0.92;
        }

        // Competition known to be circling → push harder.
        if let Some(comp) = ctx.competition_count {
            if comp >= 2 {
                offer_amount *= 1.0 + 0.04 * (comp.min(5) as f64);
            }
        }

        // Priority + scouting confidence push (Critical requests
        // back high-confidence reports → club is willing to pay).
        if let Some(req) = ctx.request {
            if req.priority == TransferNeedPriority::Critical {
                offer_amount *= 1.0 + 0.08 * aggression.max(0.3);
            }
            // Cheap / opportunistic reasons should never push up.
            if matches!(
                req.reason,
                TransferNeedReason::CheapReinforcement | TransferNeedReason::SquadPadding
            ) {
                offer_amount *= 0.9;
            }
        }
        if let Some(conf) = ctx.scout_confidence {
            if conf < self.recruitment.min_scouting_confidence {
                offer_amount *= 0.85;
            }
        }

        // Financial-stance modulation. Austerity reduces; Ambitious lifts.
        match self.recruitment.financial_stance {
            FinancialStance::Austerity => offer_amount *= 0.88,
            FinancialStance::Conservative => offer_amount *= 0.95,
            FinancialStance::Balanced => {}
            FinancialStance::Ambitious => offer_amount *= 1.05,
        }
        // Fee discipline pulls back toward the anchor.
        offer_amount *= 1.0 - fee_discipline * 0.05;

        // 3) Overpay ceiling (relative to our valuation).
        let overpay_cap = player_value.amount * self.negotiation.max_overpay_ratio as f64;
        if overpay_cap > 0.0 && offer_amount > overpay_cap {
            offer_amount = overpay_cap;
        }

        // 3b) Loan-fee hard ceiling. Whatever the anchor and the pushes
        // produced, a temporary loan fee must never approach the permanent
        // price — cap it at a small fraction of the player's full value.
        if is_loan && player_value.amount > 0.0 {
            let loan_fee_ceiling = player_value.amount * 0.20;
            if offer_amount > loan_fee_ceiling {
                offer_amount = loan_fee_ceiling;
            }
        }

        // 4) Budget cap. Critical requests + aggressive buyer get
        // closer to the whole budget.
        let mut budget_cap_ratio = 0.70 + aggression * 0.25;
        if let Some(req) = ctx.request {
            if req.priority == TransferNeedPriority::Critical {
                budget_cap_ratio += 0.10;
            } else if req.priority == TransferNeedPriority::Optional {
                budget_cap_ratio -= 0.10;
            }
        }
        let budget_cap = max_budget * budget_cap_ratio.clamp(0.30, 0.98);
        if offer_amount > budget_cap {
            offer_amount = budget_cap;
        }

        offer_amount = FormattingUtils::round_fee(offer_amount);

        let mut offer = TransferOffer::new(
            CurrencyValue {
                amount: offer_amount,
                currency: Currency::Usd,
            },
            self.club_id,
            ctx.date,
        );

        // 5) Clause construction. Each block is gated on player
        // profile and the club's preferences so a develop-and-sell
        // club, an austerity club, and an ambitious giant all
        // produce visibly different shapes for the same target.
        let age = player.age(ctx.date);

        // Assessed potential gap — hidden PA is never used: the
        // scouting context when present, the staff-free observable
        // ceiling otherwise.
        let assessed_ability =
            ctx.scout_assessed_ability
                .unwrap_or(player.player_attributes.current_ability) as i16;
        let assessed_potential = ctx
            .scout_assessed_potential
            .map(|p| p as i16)
            .unwrap_or_else(|| PotentialEstimator::observable_ceiling(player, ctx.date) as i16);
        let potential_gap = assessed_potential - assessed_ability;

        // Sell-on for young high-upside players. Stronger pull
        // for develop-and-sell clubs (protect resale).
        let wants_sell_on = age < 24
            && potential_gap > 10
            && (matches!(self.recruitment.philosophy, ClubPhilosophy::DevelopAndSell)
                || self.negotiation.sell_on_preference > 0.35);
        if wants_sell_on {
            let pct_floor = 0.08 + self.negotiation.sell_on_preference * 0.10;
            let pct_from_potential = (potential_gap as f32 / 100.0).clamp(0.0, 0.15);
            let sell_on_pct = (pct_floor + pct_from_potential).clamp(0.05, 0.25);
            offer = offer.with_clause(TransferClause::SellOnClause(sell_on_pct));
        }

        // Veteran risk shifting → appearance fees, shorter contract.
        if age > 28 {
            let appearance_amount = FormattingUtils::round_fee(
                offer_amount * (0.10 + self.negotiation.addon_preference as f64 * 0.10),
            );
            offer = offer.with_clause(TransferClause::AppearanceFee(
                CurrencyValue {
                    amount: appearance_amount,
                    currency: Currency::Usd,
                },
                20,
            ));
        }

        // Goal bonus for productive forwards.
        if player.position().is_forward() && player.statistics.goals > 5 {
            let goals_bonus = FormattingUtils::round_fee(
                offer_amount * (0.10 + self.negotiation.addon_preference as f64 * 0.10),
            );
            offer = offer.with_clause(TransferClause::GoalBonus(
                CurrencyValue {
                    amount: goals_bonus,
                    currency: Currency::Usd,
                },
                15,
            ));
        }

        // Lower-reputation buying clubs: promotion bonus is a
        // realistic carrot they can attach without raising base.
        if self.reputation_level < 60 {
            let promotion_bonus = FormattingUtils::round_fee(offer_amount * 0.18);
            offer = offer.with_clause(TransferClause::PromotionBonus(CurrencyValue {
                amount: promotion_bonus,
                currency: Currency::Usd,
            }));
        }

        // Installments — preferred by cash-poor / austerity clubs
        // and by clubs whose installment_preference is high.
        let cash_poor = ctx.buying_club_balance < 0
            || self.recruitment.financial_stance == FinancialStance::Austerity;
        let installment_pull =
            self.negotiation.installment_preference + if cash_poor { 0.3 } else { 0.0 };
        // Only attach installments above a base-fee threshold —
        // tiny deals don't need a payment plan.
        if installment_pull > 0.55 && offer_amount >= 1_500_000.0 {
            let years = if cash_poor { 4 } else { 3 };
            let installment_amount = FormattingUtils::round_fee(offer_amount * 0.55);
            offer = offer.with_clause(TransferClause::Installments(
                CurrencyValue {
                    amount: installment_amount,
                    currency: Currency::Usd,
                },
                years,
            ));
        }

        // Develop-and-sell: longer contracts for young targets
        // (resale value protection).
        let contract_years = if ctx.is_loan() {
            // Loans carry no contract length on this side; the
            // negotiation layer fills loan-specific clauses.
            1
        } else if matches!(self.recruitment.philosophy, ClubPhilosophy::DevelopAndSell) && age < 24
        {
            5
        } else if age < 24 {
            5
        } else if age < 28 {
            4
        } else if age < 32 {
            2
        } else {
            1
        };

        // Build the structured personal-terms package so execution can
        // honour the buyer's actual commitment. Loans skip the package
        // — the borrower keeps the player on the parent contract; only
        // the wage-split is set later by the execution layer.
        let mut offer = offer.with_contract_length(contract_years);
        if !ctx.is_loan() {
            let terms = PersonalTermsPackager::build(self, player, &ctx, contract_years, age);
            offer = offer.with_personal_terms(terms);
        }
        offer
    }

    // ---- Selling side ---------------------------------------

    /// Lightweight selling-side hook. Returns the club's
    /// disposition toward an incoming approach for one of its
    /// players, taking player status, age, contract, depth
    /// pressure, and rivalry into account. Negotiation /
    /// acceptance logic in the pipeline can use this as one
    /// signal among many.
    pub fn evaluate_sale(
        &self,
        player: &Player,
        date: NaiveDate,
        is_rival_buyer: bool,
        position_depth: u8,
    ) -> SellingDecision {
        let mut score = self.selling.willingness_baseline;

        if player.statuses.has(PlayerStatusType::Lst) {
            score += 0.4;
        }
        if player.statuses.has(PlayerStatusType::Req) {
            score += 0.35;
        }
        if player.statuses.has(PlayerStatusType::Unh) {
            score += 0.2;
        }
        if player.statuses.has(PlayerStatusType::Frt) {
            score += 0.3;
        }

        if let Some(contract) = player.contract.as_ref() {
            let months_remaining = ContractTiming::months_between(date, contract.expiration);
            if months_remaining <= 6 {
                score += 0.35;
            } else if months_remaining <= 12 {
                score += 0.15;
            }
        }

        let age = player.age(date);
        if age >= 31 {
            score += self.selling.sell_aging_bias * 0.3;
        }
        if position_depth >= 3 {
            score += self.selling.sell_surplus_bias * 0.25;
        }

        score += self.selling.cash_pressure * 0.3;

        if player.statuses.has(PlayerStatusType::HG) {
            score -= self.selling.keep_homegrown_bias * 0.4;
        }
        if is_rival_buyer {
            score -= self.selling.rival_resistance * 0.5;
        }

        if matches!(self.recruitment.philosophy, ClubPhilosophy::DevelopAndSell)
            && age < 24
            && score < 0.9
        {
            // Develop-and-sell clubs hold the line on young
            // assets unless the bid is otherwise compelling.
            score -= 0.1;
        }

        if score >= 0.85 {
            SellingDecision::Encourage
        } else if score >= 0.4 {
            SellingDecision::Listen
        } else {
            SellingDecision::Reject
        }
    }
}

// ============================================================
// Policy factory impls — keep the "derive from vision" logic
// attached to the struct it constructs so callers don't bump
// into floating helper functions.
// ============================================================

impl RecruitmentPolicy {
    /// Derive a realistic recruitment policy from a board's vision
    /// and the club's overall philosophy. Used by
    /// `ClubTransferStrategy::from_club_context`.
    pub fn from_vision(philosophy: &ClubPhilosophy, vision: &ClubVision) -> Self {
        let age_preference = match (philosophy, vision.youth_focus) {
            (ClubPhilosophy::DevelopAndSell, _) => AgePreference::Youth,
            (_, VisionYouthFocus::DevelopYouth) => AgePreference::Youth,
            (_, VisionYouthFocus::SignExperienced) => AgePreference::Prime,
            (ClubPhilosophy::SignToCompete, _) => AgePreference::Prime,
            _ => AgePreference::Balanced,
        };

        let resale_value_sensitivity = match philosophy {
            ClubPhilosophy::DevelopAndSell => 0.85,
            ClubPhilosophy::Balanced => 0.45,
            ClubPhilosophy::LoanFocused => 0.3,
            ClubPhilosophy::SignToCompete => 0.25,
        };

        let domestic_bias = match vision.signing_preference {
            SigningPreference::Domestic => 0.75,
            SigningPreference::ValueHunter => 0.45,
            SigningPreference::Marquee => 0.1,
            SigningPreference::Anyone => 0.3,
        };

        let min_scouting_confidence = match vision.financial_stance {
            FinancialStance::Austerity => 0.55,
            FinancialStance::Conservative => 0.45,
            FinancialStance::Balanced => 0.35,
            FinancialStance::Ambitious => 0.3,
        };

        RecruitmentPolicy {
            philosophy: philosophy.clone(),
            financial_stance: vision.financial_stance,
            signing_preference: vision.signing_preference,
            youth_focus: vision.youth_focus,
            age_preference,
            min_scouting_confidence,
            resale_value_sensitivity,
            domestic_bias,
        }
    }
}

impl NegotiationPolicy {
    /// Derive a negotiation policy from financial stance and
    /// philosophy. `buying_aggressiveness` is left at the default
    /// 0.5 — caller overrides via the strategy's reputation-based
    /// computation.
    pub fn from_vision(vision: &ClubVision, philosophy: &ClubPhilosophy) -> Self {
        let (fee_discipline, wage_discipline, max_overpay_ratio) = match vision.financial_stance {
            FinancialStance::Austerity => (0.85, 0.85, 1.10),
            FinancialStance::Conservative => (0.70, 0.70, 1.25),
            FinancialStance::Balanced => (0.50, 0.55, 1.40),
            FinancialStance::Ambitious => (0.30, 0.35, 1.80),
        };

        let installment_preference = match vision.financial_stance {
            FinancialStance::Austerity => 0.85,
            FinancialStance::Conservative => 0.65,
            FinancialStance::Balanced => 0.40,
            FinancialStance::Ambitious => 0.20,
        };

        let addon_preference = match vision.financial_stance {
            FinancialStance::Austerity => 0.75,
            FinancialStance::Conservative => 0.55,
            FinancialStance::Balanced => 0.35,
            FinancialStance::Ambitious => 0.20,
        };

        let sell_on_preference = match philosophy {
            ClubPhilosophy::DevelopAndSell => 0.75,
            ClubPhilosophy::Balanced => 0.45,
            ClubPhilosophy::LoanFocused => 0.35,
            ClubPhilosophy::SignToCompete => 0.20,
        };

        let loan_preference = match philosophy {
            ClubPhilosophy::LoanFocused => 0.8,
            ClubPhilosophy::DevelopAndSell => 0.35,
            ClubPhilosophy::Balanced => 0.4,
            ClubPhilosophy::SignToCompete => 0.15,
        };

        let risk_appetite = match vision.financial_stance {
            FinancialStance::Austerity => 0.25,
            FinancialStance::Conservative => 0.4,
            FinancialStance::Balanced => 0.55,
            FinancialStance::Ambitious => 0.75,
        };

        NegotiationPolicy {
            buying_aggressiveness: 0.5,
            wage_discipline,
            fee_discipline,
            max_overpay_ratio,
            installment_preference,
            addon_preference,
            sell_on_preference,
            loan_preference,
            risk_appetite,
        }
    }
}

impl SquadBuildingPolicy {
    pub fn from_vision(vision: &ClubVision) -> Self {
        use crate::club::board::board::LongTermGoal;

        let phase = match vision.long_term_goal {
            Some(LongTermGoal::WinLeague) | Some(LongTermGoal::WinContinental) => {
                SquadPhase::TitlePush
            }
            Some(LongTermGoal::PromotionToTopFlight) => SquadPhase::Rebuild,
            Some(LongTermGoal::Survive) => SquadPhase::Survival,
            Some(LongTermGoal::EstablishTopHalf) | Some(LongTermGoal::WinDomesticCup) => {
                SquadPhase::Consolidation
            }
            None => SquadPhase::Consolidation,
        };

        let short_term_urgency = match phase {
            SquadPhase::TitlePush => 0.85,
            SquadPhase::Survival => 0.8,
            SquadPhase::Rebuild => 0.55,
            SquadPhase::Consolidation => 0.5,
            SquadPhase::YouthCycle => 0.35,
        };

        SquadBuildingPolicy {
            phase,
            short_term_urgency,
        }
    }
}

impl SellingPolicy {
    pub fn from_vision(vision: &ClubVision, philosophy: &ClubPhilosophy) -> Self {
        let (willingness_baseline, cash_pressure) = match vision.financial_stance {
            FinancialStance::Austerity => (0.75, 0.85),
            FinancialStance::Conservative => (0.55, 0.45),
            FinancialStance::Balanced => (0.45, 0.25),
            FinancialStance::Ambitious => (0.35, 0.15),
        };

        let keep_homegrown_bias = match vision.youth_focus {
            VisionYouthFocus::DevelopYouth => 0.7,
            VisionYouthFocus::Balanced => 0.4,
            VisionYouthFocus::SignExperienced => 0.2,
        };

        let sell_aging_bias = match philosophy {
            ClubPhilosophy::DevelopAndSell => 0.75,
            ClubPhilosophy::LoanFocused => 0.6,
            ClubPhilosophy::Balanced => 0.55,
            ClubPhilosophy::SignToCompete => 0.45,
        };

        SellingPolicy {
            willingness_baseline,
            keep_homegrown_bias,
            cash_pressure,
            sell_aging_bias,
            sell_surplus_bias: 0.6,
            rival_resistance: 0.7,
        }
    }
}

/// Small date arithmetic the strategy needs that doesn't fit
/// any existing helper module — kept on its own type to avoid
/// loose top-level functions.
struct ContractTiming;

impl ContractTiming {
    fn months_between(from: NaiveDate, to: NaiveDate) -> i32 {
        let y = to.year() - from.year();
        let m = to.month() as i32 - from.month() as i32;
        let mut months = y * 12 + m;
        if to.day() < from.day() {
            months -= 1;
        }
        months.max(0)
    }
}

// ============================================================
// Personal-terms packaging — builds the [`PersonalTermsOffer`]
// from a recruitment context. Lives on its own struct so callers
// see a discoverable API and the packaging policy is unit-testable.
// ============================================================

/// Build a [`PersonalTermsOffer`] for a permanent signing. Reads the
/// buyer's negotiation/recruitment policy plus the player profile so
/// the resulting package matches the rest of the offer in tone:
///
///   - **Wage**: `WageCalculator::expected_annual_wage` anchored on
///     the buyer's tier; clamped by the buyer's wage-discipline so
///     austerity / conservative stances offer slightly less.
///   - **Signing bonus**: scales with the buyer's
///     `addon_preference` and the player's star quality.
///   - **Agent fee**: percentage of base fee for ambitious buyers
///     chasing top targets; zero for austerity sides.
///   - **Release clause**: only attached when the player has clear
///     market value and the buyer has the bargaining position to
///     accept one (or when the personal-terms policy demands it).
///   - **Squad role promise**: derived from the request reason and
///     the player's ability vs the buyer's tier.
pub struct PersonalTermsPackager;

impl PersonalTermsPackager {
    pub fn build(
        strategy: &ClubTransferStrategy,
        player: &Player,
        ctx: &TransferStrategyContext,
        contract_years: u8,
        age: u8,
    ) -> PersonalTermsOffer {
        let wage = WageCalculator::expected_annual_wage(
            player,
            age,
            ctx.buyer_reputation_score,
            ctx.league_reputation,
        );
        let wage_discount = match strategy.recruitment.financial_stance {
            FinancialStance::Austerity => 0.88,
            FinancialStance::Conservative => 0.95,
            FinancialStance::Balanced => 1.00,
            FinancialStance::Ambitious => 1.06,
        };
        let annual_wage = ((wage as f32) * wage_discount).round() as u32;

        let ca = player.player_attributes.current_ability;
        let star = ca >= 150 || player.player_attributes.world_reputation >= 6000;
        // Prospect framing reads the scouts' belief (or the observable
        // ceiling), never the hidden biological PA.
        let assessed_potential = ctx
            .scout_assessed_potential
            .unwrap_or_else(|| PotentialEstimator::observable_ceiling(player, ctx.date));
        let prospect = age <= 23 && assessed_potential as i16 - ca as i16 >= 15;

        // Signing bonus: 0–35% of annual wage depending on star quality
        // and the buyer's addon preference. Cash-poor buyers don't pay
        // them; ambitious buyers stretch.
        let signing_bonus = if ctx.buying_club_balance < 0 {
            0
        } else {
            let pct: f32 = if star {
                0.30 + 0.20 * strategy.negotiation.addon_preference
            } else if prospect {
                0.10
            } else {
                0.05
            };
            ((annual_wage as f32) * pct.clamp(0.0, 0.40)).round() as u32
        };

        // Agent fee: scales with base fee — typical real-world packages
        // are 5–10% of the transfer fee for big moves.
        let agent_fee = if strategy.recruitment.financial_stance == FinancialStance::Austerity {
            0
        } else {
            let base_fee = ctx.allocated_budget.max(0.0);
            let pct = if star { 0.08 } else { 0.04 };
            (base_fee * pct).round() as u32
        };

        // Squad-role promise: drawn from the transfer request reason +
        // player ability. Critical formation gaps imply a starter
        // promise; cheap reinforcements get rotation; prospects come
        // in as hot-prospect.
        let role_promise = Self::squad_status_promise(strategy, player, ctx, prospect, star);

        // Release clause: an ambitious buyer chasing a star pays the
        // headline number but commits to a release tag so the seller
        // can re-extract them at a premium later. Defensive/austerity
        // buyers omit. Cap at 3.5× the base fee.
        let release_clause_fee = if star && strategy.negotiation.max_overpay_ratio >= 1.5 {
            let base_fee = ctx.allocated_budget.max(0.0);
            Some((base_fee * 3.5).round() as u32)
        } else {
            None
        };

        PersonalTermsOffer {
            annual_wage: Some(annual_wage),
            signing_bonus: if signing_bonus > 0 {
                Some(signing_bonus)
            } else {
                None
            },
            agent_fee: if agent_fee > 0 { Some(agent_fee) } else { None },
            contract_years: Some(contract_years),
            squad_status_promise: role_promise,
            release_clause_fee,
        }
    }

    fn squad_status_promise(
        strategy: &ClubTransferStrategy,
        _player: &Player,
        ctx: &TransferStrategyContext,
        prospect: bool,
        star: bool,
    ) -> Option<PromisedSquadStatus> {
        if star {
            return Some(PromisedSquadStatus::KeyPlayer);
        }
        if let Some(req) = ctx.request {
            return Some(match req.reason {
                TransferNeedReason::FormationGap | TransferNeedReason::QualityUpgrade => {
                    PromisedSquadStatus::FirstTeamRegular
                }
                TransferNeedReason::DepthCover | TransferNeedReason::SquadPadding => {
                    PromisedSquadStatus::FirstTeamSquadRotation
                }
                TransferNeedReason::DevelopmentSigning => {
                    PromisedSquadStatus::HotProspectForTheFuture
                }
                TransferNeedReason::ExperiencedHead => PromisedSquadStatus::FirstTeamRegular,
                TransferNeedReason::SuccessionPlanning => PromisedSquadStatus::FirstTeamRegular,
                _ => PromisedSquadStatus::FirstTeamSquadRotation,
            });
        }
        if prospect {
            return Some(PromisedSquadStatus::HotProspectForTheFuture);
        }
        // Without a request to anchor the promise, default by tier —
        // ambitious buyers offer regular roles, others rotation.
        if matches!(
            strategy.recruitment.financial_stance,
            FinancialStance::Ambitious
        ) {
            Some(PromisedSquadStatus::FirstTeamRegular)
        } else {
            Some(PromisedSquadStatus::FirstTeamSquadRotation)
        }
    }
}
