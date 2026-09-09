use crate::club::board::{ClubVision, FinancialStance, SigningPreference, VisionYouthFocus};
use crate::club::staff::perception::PotentialEstimator;
use crate::shared::{Currency, CurrencyValue};
use crate::transfers::deal::offer::{
    PersonalTermsOffer, PromisedSquadStatus, TransferClause, TransferOffer,
};
use crate::transfers::pipeline::{
    TransferApproach, TransferNeedPriority, TransferNeedReason, TransferRequest,
};
use crate::transfers::scouting::recruitment::BoardRecruitmentDossier;
use crate::transfers::value::PlayerValuationCalculator;
use crate::transfers::value::wage::BuyerLevelWage;
use crate::utils::FormattingUtils;
use crate::{
    ClubPhilosophy, Person, Player, PlayerPositionType, PlayerSquadStatus, PlayerStatusType,
};
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
        let ca = player.player_attributes.current_ability;
        let star = ca >= 150 || player.player_attributes.world_reputation >= 6000;
        // Prospect framing reads the scouts' belief (or the observable
        // ceiling), never the hidden biological PA.
        let assessed_potential = ctx
            .scout_assessed_potential
            .unwrap_or_else(|| PotentialEstimator::observable_ceiling(player, ctx.date));
        let prospect = age <= 23 && assessed_potential as i16 - ca as i16 >= 15;

        // Squad-role promise: drawn from the transfer request reason +
        // player ability. Critical formation gaps imply a starter
        // promise; cheap reinforcements get rotation; prospects come
        // in as hot-prospect.
        //
        // Priced BEFORE the wage, because the wage is the wage for THAT
        // shirt.
        let role_promise = Self::squad_status_promise(strategy, player, ctx, prospect, star);

        // ONE wage curve, shared with the figure staged on the
        // negotiation ([`BuyerLevelWage`]).
        //
        // The packager used to price a plain `WageCalculator` figure with
        // no status premium while the negotiation staged a
        // `ContractValuation` at the PROMISED status — up to ~1.65× apart,
        // and the contract installed on completion was the packager's. A
        // Key-Player promise accepted at round 0 therefore installed a
        // wage the man had never said yes to, and the salary-happiness
        // model read him as underpaid from his first day.
        let wage = BuyerLevelWage::evaluate(
            player,
            age,
            ctx.buyer_reputation_score,
            ctx.league_reputation,
            role_promise,
        );
        // The club's own philosophy still bites — on that figure, not on a
        // parallel one.
        let wage_discount = match strategy.recruitment.financial_stance {
            FinancialStance::Austerity => 0.88,
            FinancialStance::Conservative => 0.95,
            FinancialStance::Balanced => 1.00,
            FinancialStance::Ambitious => 1.06,
        };
        let annual_wage = ((wage as f32) * wage_discount).round() as u32;

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

    /// The brief's promised role, in the shape an offer carries it.
    /// `None` for the statuses a signing is never promised (a club does not
    /// offer a man `NotNeeded`), which sends the caller to the motive-based
    /// fallback.
    fn promise_from_squad_status(status: &PlayerSquadStatus) -> Option<PromisedSquadStatus> {
        match status {
            PlayerSquadStatus::KeyPlayer => Some(PromisedSquadStatus::KeyPlayer),
            PlayerSquadStatus::FirstTeamRegular => Some(PromisedSquadStatus::FirstTeamRegular),
            PlayerSquadStatus::FirstTeamSquadRotation | PlayerSquadStatus::MainBackupPlayer => {
                Some(PromisedSquadStatus::FirstTeamSquadRotation)
            }
            PlayerSquadStatus::HotProspectForTheFuture | PlayerSquadStatus::DecentYoungster => {
                Some(PromisedSquadStatus::HotProspectForTheFuture)
            }
            _ => None,
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
        // The brief already decided what shirt this search is for — a
        // transformative signing is promised a key role, an upgrade a
        // starting one, cover a rotation seat — so the promise the buyer
        // makes is the promise it planned, not a second guess from the
        // motive. The motive mapping below stays as the fallback for
        // requests raised by paths that predate the brief.
        if let Some(req) = ctx.request {
            if let Some(promised) = Self::promise_from_squad_status(&req.promised_status) {
                return Some(promised);
            }
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
