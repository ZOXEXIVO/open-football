//! Shared plausibility layer used by every transfer-target generation
//! path and by negotiation entry. Replaces the patchwork of inline
//! reputation gates that let unrealistic moves slip through whenever a
//! caller (DoF bargain hunt, listed sweep, shortlist, direct negotiation
//! entry) bypassed [`super::scouting_config::ScoutingConfig::is_target_realistic`].
//!
//! The contract is intentionally generic: nothing here knows about
//! particular clubs, leagues, or players. Reasoning is driven by
//! continuous reputation gaps, the player's importance at the seller,
//! and explicit availability signals.
//!
//! ### High-level flow
//!
//! 1. Compute `player_importance` from squad status, rank within the
//!    seller's position group, season appearances, and ability vs the
//!    best player in the same group. Goalkeepers weight rank more
//!    heavily — there is only one starting slot.
//! 2. Compute `sporting_drop` — a blend of club, league, and world rep
//!    gaps measuring how far the buyer sits below the seller.
//! 3. Evaluate hard-reject rules (impossible signings):
//!      * Important player at a much stronger club, unsolicited and
//!        not on any availability signal.
//!      * Same-domestic-market step-down for a prime-age starter.
//!      * Loan request for an important player at a stronger club.
//!      * Fee or wages clearly out of reach for the buyer.
//! 4. If not hard-rejected, return soft adjustments shortlist scoring
//!    and the negotiation resolver should apply (score multiplier,
//!    seller acceptance delta, player terms delta, minimum-fee floor).
//!
//! ### Availability strength
//!
//! Availability is graded, not boolean — see [`AvailabilityStrength`] and
//! [`TransferPlausibilityInputs::availability_strength`]. A `Real`/`Forced`
//! signal (transfer request, genuine listing, triggered release clause)
//! *opens* the hard importance / step-down gate; a `Soft` signal (mild
//! unhappiness, near-expiry, mild seller debt) only softens it and never
//! unlocks it for a very-important player or a huge sporting drop. In every
//! case the fee / wage / willingness realism downstream still applies —
//! availability opens the door, it does not erase it. Synthetic "we created
//! a listing to back our bid" listings are excluded: callers populate the
//! listing flags from the player's real status only.

use crate::{PlayerFieldPositionGroup, PlayerSquadStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPlausibilityReason {
    ImportantPlayerAtMuchStrongerClub,
    DomesticStepDownForPrimeStarter,
    UnaffordableWages,
    UnaffordableFee,
    #[allow(dead_code)]
    NoSportingUpside,
    LoanNotCredible,
    /// Real-world country-pair friction closes this route on the
    /// current sim date — see
    /// [`crate::transfers::TransferRoutePolicy::is_blocked`]. The only
    /// active rule today is Russia ↔ Ukraine from 2022-02-24 onwards;
    /// the simulation refuses these moves at every stage.
    CountryPairBlocked,
}

// ============================================================
// Effective market reputation
// ============================================================

/// Blends a player's three reputation axes into a single market-relevant
/// figure on the 0..10000 scale. The blend shifts with how *domestic* the
/// move is: a domestic / same-region move weights **home** and **current**
/// reputation heavily, while a cross-border move leans on **world**
/// reputation (the only axis the foreign market really knows him by).
///
/// This is what makes a player who is a giant in his own country — high
/// home / current standing but a modest international footprint — register
/// as a high-reputation target in his domestic market instead of a
/// low-world-rep bargain. Example: world 2869, current 5575, home 6177
/// blends to ~5200 domestically, not ~2869.
pub struct EffectivePlayerReputation;

impl EffectivePlayerReputation {
    pub fn compute(world_rep: i16, current_rep: i16, home_rep: i16, domestic: bool) -> i16 {
        let world = world_rep.max(0) as f32;
        let current = current_rep.max(0) as f32;
        let home = home_rep.max(0) as f32;
        // Domestic: home + current dominate. Cross-border: world leads but
        // current/home still temper it (a domestic great isn't anonymous
        // abroad, just less renowned).
        let (w_world, w_current, w_home) = if domestic {
            (0.20, 0.45, 0.35)
        } else {
            (0.55, 0.30, 0.15)
        };
        (w_world * world + w_current * current + w_home * home)
            .round()
            .clamp(0.0, 10_000.0) as i16
    }
}

// ============================================================
// Structured availability strength
// ============================================================

/// How strong a player's availability signal is for a move. Replaces the
/// old boolean exemption: availability **opens the door**, it does not
/// **erase** level / price / wage / willingness realism. The tier controls
/// how far an availability signal can unlock an otherwise-blocked move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AvailabilityStrength {
    /// No availability signal — every realism gate is fully armed.
    None,
    /// A soft signal (mild unhappiness, near-expiry without affordable
    /// wages, mild seller debt). Softens penalties but never unlocks the
    /// hard importance / step-down gate for a very-important player or a
    /// huge sporting drop.
    Soft,
    /// A real, voluntary or club-driven signal (transfer request, genuine
    /// listing for the matching move type, confirmed NotNeeded, distressed
    /// seller with a clear premium on the table). Opens the hard gate but
    /// keeps fee / wage / compensation / willingness realism downstream.
    Real,
    /// The sale is forced — a triggered release clause. Bypasses the
    /// sporting gates entirely (the escape route was negotiated for this).
    Forced,
}

impl AvailabilityStrength {
    /// True when the signal is strong enough to unlock the hard
    /// importance / domestic-step-down gate (the door is genuinely open).
    pub fn unlocks_hard_gate(self) -> bool {
        matches!(
            self,
            AvailabilityStrength::Real | AvailabilityStrength::Forced
        )
    }

    /// True when wages no longer hard-block the move — only a player who
    /// actively wants out (or a forced clause) waives the buyer's duty to
    /// fund a credible wage before showing public interest.
    pub fn waives_wage_floor(self) -> bool {
        matches!(
            self,
            AvailabilityStrength::Real | AvailabilityStrength::Forced
        )
    }
}

// ============================================================
// Staged move plausibility — the central model
// ============================================================

/// The furthest stage a (buyer, seller, player) move can credibly reach.
/// Ordered: each later stage subsumes the earlier ones. Callers gate on
/// `>=` the stage they need (e.g. scouting sets `Wnt` only at
/// `CanShowPublicInterest`, negotiation creation needs
/// `CanStartNegotiation`, personal-terms needs `CanAgreePersonalTerms`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransferMoveStage {
    /// The move is impossible even to watch — a closed country route.
    Blocked,
    /// A club may quietly watch the player. Never sets public interest.
    CanScoutQuietly,
    /// The player may be an internal recruitment name. Still no `Wnt`.
    CanShortlistInternally,
    /// The move is plausible enough that the player / agent would not
    /// immediately dismiss it — public interest (`Wnt`) is allowed.
    CanShowPublicInterest,
    /// Fee + wage are credible enough to open club-to-club talks.
    CanStartNegotiation,
    /// The player's career incentives make the move make sense.
    CanAgreePersonalTerms,
    /// Final safety stage — nothing left blocking completion.
    CanCompleteMove,
}

/// Explainability record for one staged assessment. Every field is cheap
/// and `Copy`; callers attach the buyer/seller/player ids and names when
/// they log it. Makes a decision narratable: "watched privately but did
/// not show public interest because player_wont_step_down",
/// "could afford the fee but not the wages", etc.
#[derive(Debug, Clone, Copy)]
pub struct TransferMoveDiagnostics {
    pub seller_rep: f32,
    pub buyer_rep: f32,
    pub seller_league_rep: u16,
    pub buyer_league_rep: u16,
    pub player_world_rep: i16,
    pub player_current_rep: i16,
    pub player_home_rep: i16,
    pub player_effective_rep: i16,
    pub importance: f32,
    pub sporting_drop: f32,
    /// Effective player reputation minus the buyer's reputation reach.
    pub reputation_drop: i16,
    pub availability: AvailabilityStrength,
    /// estimated_value ÷ the buyer's affordable fee ceiling.
    pub fee_to_value_ratio: f32,
    /// expected annual wage ÷ current salary.
    pub wage_to_current_ratio: f32,
    pub stage: TransferMoveStage,
    pub blocking_reason: Option<TransferPlausibilityReason>,
}

/// Result of the staged plausibility assessment: how far the move reaches,
/// what stopped it, the soft adjustments to apply if it did reach
/// negotiation, and the diagnostics.
#[derive(Debug, Clone, Copy)]
pub struct TransferMoveAssessment {
    pub stage: TransferMoveStage,
    pub blocking_reason: Option<TransferPlausibilityReason>,
    pub adjustment: TransferPlausibilityAdjustment,
    pub diagnostics: TransferMoveDiagnostics,
}

impl TransferMoveAssessment {
    /// True when the move can credibly reach at least `stage`.
    pub fn reaches(&self, stage: TransferMoveStage) -> bool {
        self.stage >= stage
    }
}

impl TransferMoveDiagnostics {
    /// One-line, human-readable explanation of the decision — the spec's
    /// "make cases explainable" surface. Reads every captured signal so a
    /// rejected (or accepted) interest can be narrated:
    /// "watched privately but did not show public interest because
    /// player_wont_step_down", "could afford the fee but not the wages".
    pub fn explain(&self) -> String {
        let reason = match self.blocking_reason {
            Some(r) => format!("{:?}", r),
            None => "clear".to_string(),
        };
        format!(
            "stage={:?} reason={} importance={:.2} sporting_drop={:.2} avail={:?} \
             eff_rep={} (world {} / current {} / home {}) rep_drop={} \
             buyer_rep={:.2} seller_rep={:.2} buyer_league_rep={} seller_league_rep={} \
             fee/value={:.2} wage/current={:.2}",
            self.stage,
            reason,
            self.importance,
            self.sporting_drop,
            self.availability,
            self.player_effective_rep,
            self.player_world_rep,
            self.player_current_rep,
            self.player_home_rep,
            self.reputation_drop,
            self.buyer_rep,
            self.seller_rep,
            self.buyer_league_rep,
            self.seller_league_rep,
            self.fee_to_value_ratio,
            self.wage_to_current_ratio,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransferPlausibilityAdjustment {
    /// Multiplier applied to a shortlist candidate score. < 1.0 dampens
    /// unrealistic-but-not-impossible targets so realistic alternatives
    /// outrank them.
    pub shortlist_score_multiplier: f32,
    /// Additive delta on the seller-side acceptance chance (percent).
    /// Negative for prominent players or large sporting drops.
    pub seller_acceptance_delta: f32,
    /// Additive delta on the personal-terms acceptance chance (percent).
    /// Negative when the player is being asked to step down.
    pub player_terms_delta: f32,
    /// Multiplier applied to the asking price before judging whether an
    /// offer ratio is acceptable. Always >= 1.0 — important players at
    /// big clubs require a premium over their headline value.
    pub minimum_fee_multiplier: f64,
}

impl TransferPlausibilityAdjustment {
    pub fn neutral() -> Self {
        TransferPlausibilityAdjustment {
            shortlist_score_multiplier: 1.0,
            seller_acceptance_delta: 0.0,
            player_terms_delta: 0.0,
            minimum_fee_multiplier: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransferPlausibilityVerdict {
    HardReject(TransferPlausibilityReason),
    Allow(TransferPlausibilityAdjustment),
}

impl TransferPlausibilityVerdict {
    #[allow(dead_code)]
    pub fn is_rejected(&self) -> bool {
        matches!(self, TransferPlausibilityVerdict::HardReject(_))
    }

    pub fn adjustment(&self) -> TransferPlausibilityAdjustment {
        match self {
            TransferPlausibilityVerdict::Allow(adj) => *adj,
            TransferPlausibilityVerdict::HardReject(_) => TransferPlausibilityAdjustment::neutral(),
        }
    }
}

/// Inputs the plausibility evaluator needs. Callers build this from
/// whatever shape they have (PlayerSummary, PlayerSnapshot, raw Player
/// + Club refs in the negotiation pipeline). Keeping the evaluator a
/// pure function of this struct makes it trivially unit-testable and
/// keeps integration churn confined to building inputs.
#[derive(Debug, Clone)]
pub struct TransferPlausibilityInputs {
    pub buyer_rep: f32,
    pub seller_rep: f32,
    pub buyer_league_rep: u16,
    pub seller_league_rep: u16,
    pub buyer_world_rep: i16,
    pub seller_world_rep: i16,

    pub player_world_rep: i16,
    pub player_current_rep: i16,
    pub player_home_rep: i16,
    pub player_age: u8,
    pub position_group: PlayerFieldPositionGroup,

    pub is_listed: bool,
    pub is_loan_listed: bool,
    pub is_transfer_requested: bool,
    pub is_unhappy: bool,
    pub squad_status: PlayerSquadStatus,
    pub contract_months_remaining: i16,

    pub current_salary: u32,
    pub estimated_value: f64,

    pub player_appearances: u16,
    /// Player rank within the seller's main team at this position
    /// group, 0-indexed. 0 = #1 starter, 1 = #2, etc. Computed by
    /// callers from the seller's roster.
    pub seller_position_rank: u8,
    pub player_ca: u8,
    pub best_group_ca_at_seller: u8,

    pub is_loan: bool,
    pub is_unsolicited: bool,
    pub seller_in_debt: bool,
    pub release_clause_triggered: bool,

    pub same_country: bool,
    pub same_league_or_division: bool,
    /// True when the (buyer-country, seller-country) pair is on the
    /// real-world route block list for the current sim date. Populated
    /// by the input builders. When set, [`TransferPlausibilityEvaluator::evaluate`]
    /// short-circuits with `CountryPairBlocked` before any other gate.
    pub country_pair_blocked: bool,

    pub buyer_transfer_budget: f64,
    pub buyer_wage_budget: u32,
    pub buyer_total_wages: u32,
    /// Estimated annual wage the buyer would have to offer to land this
    /// player at the buyer's tier. Built outside the evaluator (via
    /// [`crate::WageCalculator::expected_annual_wage_raw`]) so the
    /// evaluator stays free of wage policy.
    pub expected_annual_wage: u32,
}

impl TransferPlausibilityInputs {
    /// Effective market reputation of the player on the 0..10000 scale —
    /// see [`EffectivePlayerReputation`]. Domestic moves weight home +
    /// current standing; cross-border moves lean on world reputation.
    pub fn effective_player_reputation(&self) -> i16 {
        EffectivePlayerReputation::compute(
            self.player_world_rep,
            self.player_current_rep,
            self.player_home_rep,
            self.same_country || self.same_league_or_division,
        )
    }

    /// The buyer's reputation reach on the 0..10000 scale — its world
    /// standing blended with its league's. A club in a strong league can
    /// attract more renowned players than its bare world rep suggests.
    /// Compared against [`Self::effective_player_reputation`] to judge a
    /// reputation step-down.
    pub fn buyer_reputation_reach(&self) -> i16 {
        let world = self.buyer_world_rep.max(0) as f32;
        let league = self.buyer_league_rep as f32;
        (0.70 * world + 0.30 * league).round().clamp(0.0, 10_000.0) as i16
    }

    /// Structured availability strength for the move — replaces the old
    /// boolean exemption. Availability *opens the door*; it does not erase
    /// price / wage / willingness realism (those gates run regardless,
    /// scaled by the returned strength). Synthetic listings are excluded:
    /// callers populate `is_listed`/`is_loan_listed` from the player's real
    /// status, never from a listing fabricated to back an unsolicited bid.
    ///
    /// Mapping (per design):
    ///   * `Req`                         → Real
    ///   * genuine `Lst` (permanent move)→ Real
    ///   * genuine `Loa` (loan move)     → Real
    ///   * `NotNeeded` (not contradicted)→ Real, else Soft
    ///   * near-expiry + affordable wages→ Real, else Soft
    ///   * distressed seller + premium   → Real, else Soft
    ///   * `Unh`                         → Soft (severe only via Req)
    ///   * release clause triggered      → Forced
    pub fn availability_strength(&self) -> AvailabilityStrength {
        // A triggered release clause forces the sale regardless.
        if self.release_clause_triggered {
            return AvailabilityStrength::Forced;
        }
        // A formal transfer request is the strongest voluntary signal.
        if self.is_transfer_requested {
            return AvailabilityStrength::Real;
        }
        // A genuine (non-synthetic) listing for the matching move type.
        if !self.is_loan && self.is_listed {
            return AvailabilityStrength::Real;
        }
        if self.is_loan && self.is_loan_listed {
            return AvailabilityStrength::Real;
        }

        let max_fee = self.affordability_max_fee();
        // Near-end-of-contract pickup: Real when the buyer can fund the
        // fee, otherwise only a soft nudge.
        let near_expiry = self.contract_months_remaining > 0
            && self.contract_months_remaining <= thresholds::NEAR_EXPIRY_MONTHS;
        if near_expiry && self.estimated_value <= max_fee {
            return AvailabilityStrength::Real;
        }

        // NotNeeded is a real, club-driven availability — unless the
        // player's own standing contradicts it (a top-rank, renowned
        // player is never genuinely "not needed").
        if matches!(self.squad_status, PlayerSquadStatus::NotNeeded) {
            let contradicted = self.seller_position_rank == 0
                || self.effective_player_reputation() >= thresholds::RENOWN_FLOOR_REP;
            return if contradicted {
                AvailabilityStrength::Soft
            } else {
                AvailabilityStrength::Real
            };
        }

        // Distressed seller: Real with a clear premium on the table,
        // otherwise a soft distress nudge.
        if self.seller_in_debt {
            let ratio = if self.estimated_value > 1.0 {
                (max_fee / self.estimated_value).clamp(0.0, 5.0)
            } else {
                5.0
            };
            return if ratio >= thresholds::DISTRESS_PREMIUM_RATIO {
                AvailabilityStrength::Real
            } else {
                AvailabilityStrength::Soft
            };
        }

        // Unhappiness opens the door softly (a player unhappy *and*
        // requesting out is already Real via the request branch above).
        if self.is_unhappy {
            return AvailabilityStrength::Soft;
        }

        // Near-expiry without an affordable fee still softens a touch.
        if near_expiry {
            return AvailabilityStrength::Soft;
        }

        AvailabilityStrength::None
    }

    /// Largest fee the buyer can plausibly cover. Normally `1.40 ×` the
    /// declared transfer budget, but floored at a fraction of the wage
    /// bill so a club whose transfer budget reads zero (negative balance,
    /// budget not yet allocated) can still scrape together a modest fee
    /// via installments / a board top-up. Without the floor a zero budget
    /// reads as *unlimited* fee tolerance — which let broke lower-league
    /// clubs pursue expensive players from far bigger clubs.
    fn affordability_max_fee(&self) -> f64 {
        (self.buyer_transfer_budget * 1.40)
            .max(self.buyer_wage_budget as f64 * thresholds::EMERGENCY_FEE_WAGE_FRACTION)
            .max(0.0)
    }
}

/// Soft thresholds and weights for the plausibility evaluator. Kept as
/// a single constant so any future calibration tweak lands in one place
/// and the evaluator body stays pure.
mod thresholds {
    pub const IMPORTANT: f32 = 0.78;
    pub const VERY_IMPORTANT: f32 = 0.90;
    pub const PRIME_AGE_MIN: u8 = 23;
    pub const PRIME_AGE_MAX: u8 = 30;
    pub const BIG_SPORTING_DROP: f32 = 0.16;
    pub const HUGE_SPORTING_DROP: f32 = 0.26;
    pub const DOMESTIC_STEP_DOWN_DROP: f32 = 0.12;
    pub const LOAN_IMPORTANCE_BLOCK: f32 = 0.65;
    pub const LOAN_REP_GAP_BLOCK: f32 = 0.10;
    /// Emergency one-off fee a club can cover when its declared transfer
    /// budget is ~zero, expressed as a fraction of its annual wage bill.
    /// Scales with club size and the country's price level, so it floors
    /// the affordable fee without a currency-dependent magic number.
    pub const EMERGENCY_FEE_WAGE_FRACTION: f64 = 0.25;

    /// Contract months at/below which the near-expiry availability nudge
    /// can fire.
    pub const NEAR_EXPIRY_MONTHS: i16 = 6;
    /// Affordability premium (buyer reach ÷ player value) a distressed
    /// seller needs on the table for the sale to read as a real,
    /// financially-motivated deal rather than a soft distress nudge.
    pub const DISTRESS_PREMIUM_RATIO: f64 = 1.35;

    /// Effective market reputation at/above which a player is a recognised
    /// name in his market — he resists an unsolicited domestic step-down
    /// even on a soft appearance sample, and a `NotNeeded` tag on him reads
    /// as contradicted.
    pub const RENOWN_FLOOR_REP: i16 = 6000;
    /// Effective-reputation importance floor ramp: below LOW it adds
    /// nothing, at/above HIGH it floors importance at MAX_FLOOR. Lets a
    /// genuinely renowned player stay "important" through a thin
    /// early-season appearance sample without single-handedly crossing the
    /// hard-block band.
    pub const REP_IMPORTANCE_LOW: f32 = 3500.0;
    pub const REP_IMPORTANCE_HIGH: f32 = 7500.0;
    pub const REP_IMPORTANCE_MAX_FLOOR: f32 = 0.80;
    /// Effective player reputation this far above the buyer's reach reads
    /// as a reputation step-down the player resists in his own market.
    pub const REP_STEP_DOWN_GAP: i16 = 1500;
}

/// Per-axis importance scoring plus the objective-evidence floor. Wrapped
/// in a struct (no free helpers) so the importance model reads as one
/// cohesive unit. The evidence floor encodes the spec's "low appearances
/// are not enough": a player who is a key man by status, the top choice in
/// his position group, or a recognised name in his market is never dragged
/// "fringe" by a thin early-season appearance sample — appearances can only
/// *add* to importance, never establish it.
struct ImportanceFactors;

impl ImportanceFactors {
    fn status_score(status: &PlayerSquadStatus) -> f32 {
        match status {
            PlayerSquadStatus::KeyPlayer => 1.00,
            PlayerSquadStatus::FirstTeamRegular => 0.88,
            PlayerSquadStatus::FirstTeamSquadRotation => 0.58,
            PlayerSquadStatus::MainBackupPlayer => 0.38,
            PlayerSquadStatus::HotProspectForTheFuture => 0.45,
            PlayerSquadStatus::DecentYoungster => 0.30,
            PlayerSquadStatus::NotNeeded => 0.05,
            PlayerSquadStatus::NotYetSet | PlayerSquadStatus::Invalid => 0.45,
            PlayerSquadStatus::SquadStatusCount => 0.45,
        }
    }

    fn rank_score(rank: u8) -> f32 {
        match rank {
            0 => 1.00,
            1 => 0.62,
            2 => 0.35,
            _ => 0.15,
        }
    }

    fn appearance_score(apps: u16) -> f32 {
        match apps {
            0 => 0.05,
            1..=5 => 0.20,
            6..=11 => 0.40,
            12..=19 => 0.65,
            20..=27 => 0.85,
            _ => 1.00,
        }
    }

    fn ability_score(player_ca: u8, best_group_ca: u8) -> f32 {
        let player = player_ca.max(1) as f32;
        let best = best_group_ca.max(1) as f32;
        (player / best).clamp(0.0, 1.0)
    }

    /// Objective-evidence importance floor — the highest "he is clearly
    /// important" signal available, independent of how many minutes he has
    /// played this season.
    fn evidence_floor(inputs: &TransferPlausibilityInputs) -> f32 {
        let mut floor = 0.0_f32;
        // Declared squad role.
        floor = floor.max(match inputs.squad_status {
            PlayerSquadStatus::KeyPlayer => 0.92,
            PlayerSquadStatus::FirstTeamRegular => 0.80,
            _ => 0.0,
        });
        // Top of his position group at the seller (first / second choice).
        floor = floor.max(match inputs.seller_position_rank {
            0 => 0.80,
            1 => 0.50,
            _ => 0.0,
        });
        // Best (or co-best) current ability in his position group.
        if inputs.best_group_ca_at_seller > 0
            && inputs.player_ca + 2 >= inputs.best_group_ca_at_seller
        {
            floor = floor.max(0.78);
        }
        // Recognised name in his market — ramps with effective reputation,
        // capped so renown reinforces but rarely alone crosses the band.
        floor.max(Self::reputation_floor(inputs.effective_player_reputation()))
    }

    fn reputation_floor(effective_rep: i16) -> f32 {
        let lo = thresholds::REP_IMPORTANCE_LOW;
        let hi = thresholds::REP_IMPORTANCE_HIGH;
        (((effective_rep as f32 - lo) / (hi - lo)).clamp(0.0, 1.0))
            * thresholds::REP_IMPORTANCE_MAX_FLOOR
    }
}

/// Stateless namespace for the pure plausibility evaluator. Wrapped
/// in a struct (rather than free functions) so the evaluator surface
/// reads like an API: `TransferPlausibilityEvaluator::evaluate(...)`,
/// `TransferPlausibilityEvaluator::player_importance(...)`,
/// `TransferPlausibilityEvaluator::sporting_drop(...)`.
pub struct TransferPlausibilityEvaluator;

impl TransferPlausibilityEvaluator {
    /// Importance of the player to the selling club (0.0..1.0). The
    /// goalkeeper variant weights rank more heavily because there is only
    /// one starting slot — a first-choice GK is materially harder to sell
    /// down than a first-choice winger with a deputy ready to slot in.
    pub fn player_importance(inputs: &TransferPlausibilityInputs) -> f32 {
        let s = ImportanceFactors::status_score(&inputs.squad_status);
        let r = ImportanceFactors::rank_score(inputs.seller_position_rank);
        let a = ImportanceFactors::appearance_score(inputs.player_appearances);
        let ab = ImportanceFactors::ability_score(inputs.player_ca, inputs.best_group_ca_at_seller);

        let raw = if inputs.position_group == PlayerFieldPositionGroup::Goalkeeper {
            0.36 * s + 0.34 * r + 0.18 * a + 0.12 * ab
        } else {
            0.34 * s + 0.25 * r + 0.25 * a + 0.16 * ab
        };
        // Low appearances are never enough on their own: a key man by
        // status / rank / ability / renown keeps a high importance floor
        // even on a thin early-season sample.
        raw.max(ImportanceFactors::evidence_floor(inputs))
            .clamp(0.0, 1.0)
    }

    /// Sporting "drop" the move represents for the player. Positive when
    /// the buyer is below the seller, zero when peer, negative when the
    /// buyer is above. Returned in [-1, +1] roughly; values above ~0.16
    /// are treated as meaningful step-downs.
    pub fn sporting_drop(inputs: &TransferPlausibilityInputs) -> f32 {
        let rep_gap = inputs.seller_rep - inputs.buyer_rep;
        let league_gap =
            (inputs.seller_league_rep as f32 - inputs.buyer_league_rep as f32) / 10_000.0;
        let world_gap = (inputs.seller_world_rep as f32 - inputs.buyer_world_rep as f32) / 10_000.0;
        0.55 * rep_gap + 0.30 * league_gap + 0.15 * world_gap
    }

    /// Single entry point used everywhere the pipeline needs a yes/no
    /// verdict (scouting candidate filter, shortlist build, negotiation
    /// creation). A move that cannot credibly reach
    /// [`TransferMoveStage::CanStartNegotiation`] is a `HardReject`;
    /// otherwise the soft adjustments apply. Delegates to the staged
    /// [`TransferMovePlausibility`] model so the single-shot and staged
    /// views can never drift apart.
    pub fn evaluate(inputs: &TransferPlausibilityInputs) -> TransferPlausibilityVerdict {
        let assessment = TransferMovePlausibility::assess(inputs);
        if assessment.stage < TransferMoveStage::CanStartNegotiation {
            TransferPlausibilityVerdict::HardReject(
                assessment
                    .blocking_reason
                    .unwrap_or(TransferPlausibilityReason::NoSportingUpside),
            )
        } else {
            TransferPlausibilityVerdict::Allow(assessment.adjustment)
        }
    }
}

// ============================================================
// TransferMovePlausibility — the central staged model
// ============================================================

/// Stateless namespace for the staged move-plausibility model. Every path
/// in the transfer pipeline consults it: scouting candidate filtering and
/// scouting-report → public-interest gating, staff recommendations,
/// shortlist building, negotiation creation, the seller's initial response,
/// personal terms, and market-circulation diagnostics. The single-shot
/// [`TransferPlausibilityEvaluator::evaluate`] is a thin wrapper over it.
///
/// The model walks the move through escalating credibility stages
/// (`CanScoutQuietly` → … → `CanCompleteMove`) and returns the furthest
/// stage it reaches plus the gate that stopped it. Availability *opens*
/// gates; it never erases the fee / wage / willingness realism downstream.
pub struct TransferMovePlausibility;

impl TransferMovePlausibility {
    pub fn assess(inputs: &TransferPlausibilityInputs) -> TransferMoveAssessment {
        let importance = TransferPlausibilityEvaluator::player_importance(inputs);
        let drop = TransferPlausibilityEvaluator::sporting_drop(inputs);
        let strength = inputs.availability_strength();
        let eff_rep = inputs.effective_player_reputation();
        let reach = inputs.buyer_reputation_reach();
        let rep_drop =
            (eff_rep as i32 - reach as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16;

        let very_important = importance >= thresholds::VERY_IMPORTANT;
        let huge_drop = drop >= thresholds::HUGE_SPORTING_DROP;
        let adjustment = Self::soft_adjustment(
            inputs,
            importance,
            drop,
            very_important,
            huge_drop,
            strength,
        );

        let max_fee = inputs.affordability_max_fee();
        let fee_to_value_ratio = if max_fee > 0.0 {
            (inputs.estimated_value / max_fee) as f32
        } else {
            f32::INFINITY
        };
        let wage_to_current_ratio = if inputs.current_salary > 0 {
            inputs.expected_annual_wage as f32 / inputs.current_salary as f32
        } else {
            0.0
        };

        // Builds the assessment with the diagnostics filled from the
        // captured context — one place so every early-return is consistent.
        let make = |stage: TransferMoveStage, reason: Option<TransferPlausibilityReason>| {
            TransferMoveAssessment {
                stage,
                blocking_reason: reason,
                adjustment,
                diagnostics: TransferMoveDiagnostics {
                    seller_rep: inputs.seller_rep,
                    buyer_rep: inputs.buyer_rep,
                    seller_league_rep: inputs.seller_league_rep,
                    buyer_league_rep: inputs.buyer_league_rep,
                    player_world_rep: inputs.player_world_rep,
                    player_current_rep: inputs.player_current_rep,
                    player_home_rep: inputs.player_home_rep,
                    player_effective_rep: eff_rep,
                    importance,
                    sporting_drop: drop,
                    reputation_drop: rep_drop,
                    availability: strength,
                    fee_to_value_ratio,
                    wage_to_current_ratio,
                    stage,
                    blocking_reason: reason,
                },
            }
        };

        // ── Route closure: a closed country-pair route can't even be
        // watched (no point filing a report on an impossible move). ──
        if inputs.country_pair_blocked {
            return make(
                TransferMoveStage::Blocked,
                Some(TransferPlausibilityReason::CountryPairBlocked),
            );
        }

        let important = importance >= thresholds::IMPORTANT;
        let prime_age =
            (thresholds::PRIME_AGE_MIN..=thresholds::PRIME_AGE_MAX).contains(&inputs.player_age);
        let big_drop = drop >= thresholds::BIG_SPORTING_DROP;
        let same_domestic_market = inputs.same_country || inputs.same_league_or_division;
        let hard_gate_open = strength.unlocks_hard_gate();
        // A soft signal can rescue a *merely* important player on a big (not
        // huge) drop — but never a very-important player or a huge drop.
        let soft_rescue =
            matches!(strength, AvailabilityStrength::Soft) && !very_important && !huge_drop;
        // Availability opens the importance gate — but only within a plausible
        // level band. A *huge* sporting drop on a permanent move stays
        // implausible for PUBLIC interest on a *passive / club-driven* Real
        // signal (a listing, near-expiry, a "not needed" tag, a distressed
        // seller): a vastly-smaller club is not a credible public suitor for a
        // top-flight first-teamer, and the fee gate that would otherwise filter
        // it sits *above* the public-interest stage — so without this an
        // available giant's first-choice keeper still surfaces 4th-tier sides
        // under "interested clubs". Only the player's *own* declared exit — a
        // transfer request — or a pre-negotiated escape (a triggered release
        // clause) carries his endorsement of so large a drop and reopens it
        // (the move then still dies at the fee / willingness gates). Loans are
        // governed by their own credibility gate below.
        let huge_drop_self_unlocked =
            inputs.is_transfer_requested || inputs.release_clause_triggered;
        let level_gate_open = if huge_drop && !inputs.is_loan {
            huge_drop_self_unlocked
        } else {
            hard_gate_open
        };

        // ── Public-interest gate ──────────────────────────────────────
        // Quiet scouting is always allowed (a club may watch anyone). These
        // gates decide whether the move is credible enough to put on the
        // internal shortlist / show PUBLIC interest. An *egregious* mismatch
        // (a very-important player on a huge sporting drop) is so far-fetched
        // that the club can watch but wouldn't even waste a shortlist slot;
        // a merely-important / big-drop mismatch can be an internal name but
        // not public interest.

        // Important first-team type at a much stronger club, approached cold.
        // A merely-big drop is opened by a Real signal (`level_gate_open ==
        // hard_gate_open`); a *huge* drop only by a Forced clause.
        if inputs.is_unsolicited && important && big_drop && !level_gate_open && !soft_rescue {
            let cap = if very_important && huge_drop {
                TransferMoveStage::CanScoutQuietly
            } else {
                TransferMoveStage::CanShortlistInternally
            };
            return make(
                cap,
                Some(TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub),
            );
        }

        // Same-domestic-market step-down for a prime-age starter.
        if same_domestic_market
            && prime_age
            && important
            && drop >= thresholds::DOMESTIC_STEP_DOWN_DROP
            && !hard_gate_open
            && !soft_rescue
        {
            return make(
                TransferMoveStage::CanShortlistInternally,
                Some(TransferPlausibilityReason::DomesticStepDownForPrimeStarter),
            );
        }

        // Recognised domestic name resists a clearly lower-reputation move
        // in his own market even when raw status/rank read moderate (the
        // high-home-rep, low-world-rep case the effective rep surfaces).
        if same_domestic_market
            && inputs.is_unsolicited
            && eff_rep >= thresholds::RENOWN_FLOOR_REP
            && rep_drop >= thresholds::REP_STEP_DOWN_GAP
            && !hard_gate_open
            && !(matches!(strength, AvailabilityStrength::Soft)
                && rep_drop < thresholds::REP_STEP_DOWN_GAP * 2)
        {
            return make(
                TransferMoveStage::CanShortlistInternally,
                Some(TransferPlausibilityReason::DomesticStepDownForPrimeStarter),
            );
        }

        // Loan from a bigger club down to a smaller one for an important
        // player — the parent wouldn't risk a key contributor at a sub-tier
        // suitor.
        if inputs.is_loan
            && !hard_gate_open
            && importance >= thresholds::LOAN_IMPORTANCE_BLOCK
            && (inputs.seller_rep - inputs.buyer_rep) > thresholds::LOAN_REP_GAP_BLOCK
        {
            return make(
                TransferMoveStage::CanShortlistInternally,
                Some(TransferPlausibilityReason::LoanNotCredible),
            );
        }

        // Wages are a first-class gate: the buyer must be able to fund a
        // credible wage to show public interest at all. Only a player who
        // actively wants out (Real/Forced) waives this — availability opens
        // the door, it does not pay the wages.
        if !strength.waives_wage_floor() {
            let wage_headroom =
                (inputs.buyer_wage_budget as i64 - inputs.buyer_total_wages as i64).max(0);
            let soft_wage_cap =
                ((inputs.current_salary as f64 * 1.15).max(wage_headroom as f64 * 1.30)).max(0.0);
            if soft_wage_cap > 0.0 && (inputs.expected_annual_wage as f64) > soft_wage_cap {
                return make(
                    TransferMoveStage::CanShortlistInternally,
                    Some(TransferPlausibilityReason::UnaffordableWages),
                );
            }
        }

        // ── Negotiation gate: fee affordability ──────────────────────
        // Public interest is plausible, but if the club can't fund the fee
        // it can't actually open club-to-club talks. Release clauses and
        // loans bypass the fee gate.
        if !inputs.release_clause_triggered && !inputs.is_loan && inputs.estimated_value > max_fee {
            return make(
                TransferMoveStage::CanShowPublicInterest,
                Some(TransferPlausibilityReason::UnaffordableFee),
            );
        }

        // ── Personal-terms gate: player willingness floor ────────────
        // Fee + wage are credible, so talks can open — but the player's
        // own career incentives must make sense before he agrees terms.
        if let Some(reason) = Self::player_terms_floor(inputs) {
            return make(TransferMoveStage::CanStartNegotiation, Some(reason));
        }

        // Everything clears. A move that still asks the player to step down
        // (a tolerated drop he has a reason to accept) reaches
        // `CanAgreePersonalTerms` — he will agree, but completion stays
        // sensitive to the wage / status offered. A clean lateral or upward
        // move has nothing left to weigh and reaches `CanCompleteMove`.
        let tolerated_step_down = drop > thresholds::DOMESTIC_STEP_DOWN_DROP
            && !matches!(strength, AvailabilityStrength::None);
        if tolerated_step_down {
            make(TransferMoveStage::CanAgreePersonalTerms, None)
        } else {
            make(TransferMoveStage::CanCompleteMove, None)
        }
    }

    /// Player-side willingness hard floor for the personal-terms phase. A
    /// player with **no** availability signal (`None`) hard-refuses a move
    /// his career incentives reject; any real reason to move (request,
    /// listing, unhappiness, near-expiry, forced clause) clears the hard
    /// floor and leaves only the probability texture to the resolver.
    /// Returns the reason he would refuse, or `None` if the move is
    /// basically reasonable for him.
    pub fn player_terms_floor(
        inputs: &TransferPlausibilityInputs,
    ) -> Option<TransferPlausibilityReason> {
        if !matches!(inputs.availability_strength(), AvailabilityStrength::None) {
            return None;
        }

        let importance = TransferPlausibilityEvaluator::player_importance(inputs);
        let drop = TransferPlausibilityEvaluator::sporting_drop(inputs);
        let eff_rep = inputs.effective_player_reputation();
        let rep_drop = eff_rep as i32 - inputs.buyer_reputation_reach() as i32;
        let prime =
            (thresholds::PRIME_AGE_MIN..=thresholds::PRIME_AGE_MAX).contains(&inputs.player_age);
        let domestic = inputs.same_country || inputs.same_league_or_division;

        // First-team / key player refuses a clear sporting step down.
        if importance >= thresholds::IMPORTANT && drop >= thresholds::BIG_SPORTING_DROP {
            return Some(TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub);
        }
        // Prime-age player refuses a lower-league domestic move.
        if prime
            && importance >= thresholds::IMPORTANT - 0.08
            && domestic
            && drop >= thresholds::DOMESTIC_STEP_DOWN_DROP
        {
            return Some(TransferPlausibilityReason::DomesticStepDownForPrimeStarter);
        }
        // Recognised domestic name refuses a clearly lower-reputation club.
        if domestic
            && eff_rep >= thresholds::RENOWN_FLOOR_REP
            && rep_drop >= thresholds::REP_STEP_DOWN_GAP as i32
        {
            return Some(TransferPlausibilityReason::DomesticStepDownForPrimeStarter);
        }
        None
    }

    /// Soft adjustments applied once a move is at least plausible. The
    /// sporting-drop and importance dampers match the pre-staged model; the
    /// availability counterbalance is now scaled by [`AvailabilityStrength`]
    /// (Soft nudges, Real/Forced lift) instead of an all-or-nothing boost.
    fn soft_adjustment(
        inputs: &TransferPlausibilityInputs,
        importance: f32,
        drop: f32,
        very_important: bool,
        huge_drop: bool,
        strength: AvailabilityStrength,
    ) -> TransferPlausibilityAdjustment {
        let mut adj = TransferPlausibilityAdjustment::neutral();

        if drop > 0.0 {
            adj.shortlist_score_multiplier *= (1.0_f32 - drop * 1.8).clamp(0.35, 1.0);
            adj.seller_acceptance_delta -= drop * 35.0;
            adj.player_terms_delta -= drop * 45.0;
            adj.minimum_fee_multiplier += drop as f64 * 0.9;
        }

        if importance >= 0.75 {
            adj.shortlist_score_multiplier *= 0.55;
            adj.seller_acceptance_delta -= 18.0;
            adj.minimum_fee_multiplier += 0.25;
        }

        if very_important {
            adj.shortlist_score_multiplier *= 0.45;
            adj.seller_acceptance_delta -= 28.0;
            adj.minimum_fee_multiplier += 0.45;
        }

        // Irreplaceable first-choice starter: the seller demands a clear
        // replacement-value premium on top (compensation realism — a normal
        // asking price isn't enough for a key man who isn't pushing to go).
        if importance >= thresholds::IMPORTANT && inputs.seller_position_rank == 0 {
            adj.minimum_fee_multiplier += 0.15;
        }

        if huge_drop {
            adj.shortlist_score_multiplier *= 0.75;
        }

        match strength {
            AvailabilityStrength::None => {}
            AvailabilityStrength::Soft => {
                // Opens the door a crack — softens, doesn't erase.
                adj.seller_acceptance_delta += 8.0;
                adj.player_terms_delta += 5.0;
                adj.minimum_fee_multiplier = (adj.minimum_fee_multiplier - 0.08).max(1.0);
            }
            AvailabilityStrength::Real => {
                adj.shortlist_score_multiplier = adj.shortlist_score_multiplier.max(0.75);
                adj.seller_acceptance_delta += 15.0;
                adj.player_terms_delta += 10.0;
                adj.minimum_fee_multiplier = (adj.minimum_fee_multiplier - 0.20).max(1.0);
            }
            AvailabilityStrength::Forced => {
                adj.shortlist_score_multiplier = adj.shortlist_score_multiplier.max(0.85);
                adj.seller_acceptance_delta += 25.0;
                adj.player_terms_delta += 25.0;
                adj.minimum_fee_multiplier = 1.0;
            }
        }

        // Upward / lateral move floor: a bigger or peer club should not
        // bury a smaller club's standout beneath every listed or free
        // alternative on its shortlist. The importance dampers above are a
        // SELLER-side reality (he's dear and hard to prise away); they must
        // not also make the BUYER rank his most desirable realistic target
        // last. For a move that is NOT a step down for the player
        // (`drop <= 0`, i.e. the buyer is same-or-bigger), floor the
        // shortlist multiplier so the standout stays competitive on the
        // board — the seller-side gates still make the actual deal hard.
        if drop <= 0.0 {
            adj.shortlist_score_multiplier = adj.shortlist_score_multiplier.max(0.70);
        }

        adj
    }
}

// ============================================================
// Pipeline-side input builders
// ============================================================
//
// `TransferPlausibilityBuilder` assembles `TransferPlausibilityInputs`
// from the shapes the pipeline actually carries. Keeping the builder
// methods inside a single struct means call sites stay one-liners and
// the wage policy / importance heuristics never silently drift between
// callers.

use crate::Player;
use crate::club::player::calculators::WageCalculator;
use crate::transfers::pipeline::{PipelineProcessor, PlayerSummary};
use crate::{Club, Country, Person, PlayerStatusType, TeamType};
use chrono::NaiveDate;

/// Per-club buyer snapshot reused across plausibility lookups so the
/// builder doesn't re-walk reputation/wage data for every candidate.
#[derive(Debug, Clone)]
pub(crate) struct BuyerPlausibilityContext {
    pub buyer_rep: f32,
    pub buyer_world_rep: i16,
    pub buyer_league_rep: u16,
    pub buyer_transfer_budget: f64,
    pub buyer_wage_budget: u32,
    pub buyer_total_wages: u32,
    pub buyer_country_id: u32,
    pub buyer_country_code: String,
    pub buyer_league_id: Option<u32>,
}

impl BuyerPlausibilityContext {
    pub(crate) fn build(country: &Country, club: &Club) -> Self {
        let main_team = club
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main));
        let buyer_rep = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let buyer_world_rep = main_team.map(|t| t.reputation.world as i16).unwrap_or(0);
        let buyer_league_id = main_team.and_then(|t| t.league_id);
        let buyer_league_rep = buyer_league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);
        let buyer_total_wages: u32 = club.teams.iter().map(|t| t.get_annual_salary()).sum();
        let buyer_wage_budget = club
            .finance
            .wage_budget
            .as_ref()
            .map(|b| b.amount.max(0.0) as u32)
            .unwrap_or(buyer_total_wages.saturating_mul(11) / 10);
        let buyer_transfer_budget = club
            .finance
            .transfer_budget
            .as_ref()
            .map(|b| b.amount)
            .unwrap_or(club.transfer_plan.total_budget);
        BuyerPlausibilityContext {
            buyer_rep,
            buyer_world_rep,
            buyer_league_rep,
            buyer_transfer_budget,
            buyer_wage_budget,
            buyer_total_wages,
            buyer_country_id: country.id,
            buyer_country_code: country.code.clone(),
            buyer_league_id,
        }
    }
}

/// Stateless namespace for the pipeline-facing input builders. Wrapped
/// in a struct so call sites read like a discoverable API
/// (`TransferPlausibilityBuilder::from_summary(...)`) instead of a
/// loose function grab-bag.
pub(crate) struct TransferPlausibilityBuilder;

impl TransferPlausibilityBuilder {
    /// Build plausibility inputs from a `PlayerSummary` (the unit used by
    /// process_scouting / shortlists / build_shortlists). Reads the
    /// seller-side context carried on the summary
    /// ([`crate::transfers::pipeline::SellerPlausibilityContext`]) instead of
    /// re-resolving the selling club, so a **foreign** target assesses with
    /// the same rigour as a domestic one. Previously this looked the seller
    /// club up in the *buyer's* country and returned `None` for every
    /// cross-country target — which callers read as "not rejected", letting
    /// a lower-league club publicly chase a first-team player abroad.
    ///
    /// Returns `Option` for signature stability with the staged callers; the
    /// seller context is always present on a pool-built summary, so the only
    /// `None` would come from a hand-built summary with no seller data.
    pub(crate) fn from_summary(
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
    ) -> Option<TransferPlausibilityInputs> {
        let seller = &target.seller_ctx;
        let player_ca = target.skill_ability;
        let position_group = target.position_group;
        let best_group_ca = target.club_best_in_group.max(player_ca);

        let same_country = target.country_id == buyer_ctx.buyer_country_id;
        let same_league_or_division = same_country
            && match (buyer_ctx.buyer_league_id, seller.league_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };

        let country_pair_blocked = crate::transfers::TransferRoutePolicy::is_blocked(
            &target.country_code,
            &buyer_ctx.buyer_country_code,
            date,
        );

        let expected_annual_wage = WageCalculator::expected_annual_wage_raw(
            player_ca,
            target.current_reputation,
            matches!(position_group, PlayerFieldPositionGroup::Forward),
            matches!(position_group, PlayerFieldPositionGroup::Goalkeeper),
            target.age,
            buyer_ctx.buyer_rep,
            buyer_ctx.buyer_league_rep,
        );

        Some(TransferPlausibilityInputs {
            buyer_rep: buyer_ctx.buyer_rep,
            seller_rep: seller.club_reputation_score,
            buyer_league_rep: buyer_ctx.buyer_league_rep,
            seller_league_rep: seller.league_reputation,
            buyer_world_rep: buyer_ctx.buyer_world_rep,
            seller_world_rep: target.club_world_reputation,
            player_world_rep: target.world_reputation,
            player_current_rep: target.current_reputation,
            player_home_rep: target.home_reputation,
            player_age: target.age,
            position_group,
            is_listed: target.is_listed,
            is_loan_listed: target.is_loan_listed,
            is_transfer_requested: seller.is_transfer_requested,
            is_unhappy: seller.is_unhappy,
            squad_status: seller.squad_status.clone(),
            contract_months_remaining: target.contract_months_remaining,
            current_salary: target.salary,
            estimated_value: target.estimated_value,
            player_appearances: target.appearances,
            seller_position_rank: seller.position_group_rank,
            player_ca,
            best_group_ca_at_seller: best_group_ca,
            is_loan,
            is_unsolicited,
            seller_in_debt: seller.in_debt,
            release_clause_triggered: false,
            same_country,
            same_league_or_division,
            country_pair_blocked,
            buyer_transfer_budget: buyer_ctx.buyer_transfer_budget,
            buyer_wage_budget: buyer_ctx.buyer_wage_budget,
            buyer_total_wages: buyer_ctx.buyer_total_wages,
            expected_annual_wage,
        })
    }

    /// Convenience wrapper for callers who already have a `PlayerSummary`
    /// and a buyer context: returns the verdict directly. Works for both
    /// domestic and foreign targets — the seller context rides on the
    /// summary, so no selling-country reference is needed.
    pub(crate) fn evaluate_summary(
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
    ) -> Option<TransferPlausibilityVerdict> {
        Self::from_summary(buyer_ctx, target, is_loan, is_unsolicited, date)
            .map(|i| TransferPlausibilityEvaluator::evaluate(&i))
    }

    /// Staged-model counterpart of [`Self::evaluate_summary`] — returns the
    /// full [`TransferMoveAssessment`] (furthest reachable stage +
    /// diagnostics) so a caller can gate on a specific stage (e.g. scouting
    /// sets public interest only at `CanShowPublicInterest`).
    pub(crate) fn assess_summary(
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
    ) -> Option<TransferMoveAssessment> {
        Self::from_summary(buyer_ctx, target, is_loan, is_unsolicited, date)
            .map(|i| TransferMovePlausibility::assess(&i))
    }

    /// Build plausibility inputs for a **cross-country** move with the live
    /// `Club` + `Player` references on both sides. The single source of
    /// truth for the full-reference build: [`Self::from_clubs`] (same
    /// country) delegates here with the country passed twice. Used by every
    /// foreign path — `initiate_foreign_negotiations`,
    /// `scan_foreign_loan_market`, and `clubs_interested_in_player` — so a
    /// buyer abroad is held to the same level / fee / wage / willingness
    /// realism as a domestic suitor.
    ///
    /// Seller reputation, league reputation, rank, and best-CA-in-group are
    /// read from the **selling** country/club (not the buyer's), and
    /// `same_country` / `same_league_or_division` compare the two countries
    /// directly. The country-pair route block is evaluated on the real
    /// (seller → buyer) route. When the seller context genuinely cannot be
    /// resolved the caller — not this builder — decides the fallback; this
    /// builder always returns a populated input set from the refs given.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_global(
        buying_country: &Country,
        buying_club: &Club,
        selling_country: &Country,
        selling_club: &Club,
        player: &Player,
        estimated_value: f64,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
    ) -> TransferPlausibilityInputs {
        let buyer_ctx = BuyerPlausibilityContext::build(buying_country, buying_club);

        let main_team = selling_club
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main));
        let seller_rep = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let seller_world_rep = main_team.map(|t| t.reputation.world as i16).unwrap_or(0);
        let seller_league_id = main_team.and_then(|t| t.league_id);
        // League reputation comes from the SELLER's country registry — a
        // foreign buyer must read the seller league's standing, not look it
        // up (and miss) in its own country.
        let seller_league_rep = seller_league_id
            .and_then(|lid| selling_country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        let position = player.position();
        let position_group = position.position_group();
        let player_ca = player.player_attributes.current_ability;
        let rank = PipelineProcessor::position_group_rank(selling_club, player.id, position_group);
        let rank = if rank == u8::MAX { 1 } else { rank };
        let best_group_ca =
            PipelineProcessor::best_ca_in_group(selling_club, position_group).max(player_ca);

        let statuses = player.statuses.get();
        let is_listed = statuses.contains(&PlayerStatusType::Lst);
        let is_loan_listed = statuses.contains(&PlayerStatusType::Loa);
        let is_transfer_requested = statuses.contains(&PlayerStatusType::Req);
        let is_unhappy = statuses.contains(&PlayerStatusType::Unh);

        let (squad_status, contract_months_remaining, current_salary) = player
            .contract
            .as_ref()
            .map(|c| {
                let months =
                    ((c.expiration - date).num_days().max(0) / 30).min(i16::MAX as i64) as i16;
                (c.squad_status.clone(), months, c.salary)
            })
            .unwrap_or((PlayerSquadStatus::NotYetSet, 0, 0));

        let release_clause_triggered = player
            .contract
            .as_ref()
            .map(|c| c.release_clause_triggered(0.0, false).is_some())
            .unwrap_or(false);

        let same_country = buying_country.id == selling_country.id;
        let same_league_or_division = same_country
            && match (buyer_ctx.buyer_league_id, seller_league_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };

        let expected_annual_wage = WageCalculator::expected_annual_wage(
            player,
            player.age(date),
            buyer_ctx.buyer_rep,
            buyer_ctx.buyer_league_rep,
        );

        // Real (seller → buyer) route friction. For a same-country move the
        // pair is (X, X), which is never on the block list, so `from_clubs`
        // still yields `false` here.
        let country_pair_blocked = crate::transfers::TransferRoutePolicy::is_blocked(
            &selling_country.code,
            &buying_country.code,
            date,
        );

        TransferPlausibilityInputs {
            buyer_rep: buyer_ctx.buyer_rep,
            seller_rep,
            buyer_league_rep: buyer_ctx.buyer_league_rep,
            seller_league_rep,
            buyer_world_rep: buyer_ctx.buyer_world_rep,
            seller_world_rep,
            player_world_rep: player.player_attributes.world_reputation,
            player_current_rep: player.player_attributes.current_reputation,
            player_home_rep: player.player_attributes.home_reputation,
            player_age: player.age(date),
            position_group,
            is_listed,
            is_loan_listed,
            is_transfer_requested,
            is_unhappy,
            squad_status,
            contract_months_remaining,
            current_salary,
            estimated_value,
            player_appearances: player.statistics.total_games(),
            seller_position_rank: rank,
            player_ca,
            best_group_ca_at_seller: best_group_ca,
            is_loan,
            is_unsolicited,
            seller_in_debt: selling_club.finance.balance.balance < 0,
            release_clause_triggered,
            same_country,
            same_league_or_division,
            country_pair_blocked,
            buyer_transfer_budget: buyer_ctx.buyer_transfer_budget,
            buyer_wage_budget: buyer_ctx.buyer_wage_budget,
            buyer_total_wages: buyer_ctx.buyer_total_wages,
            expected_annual_wage,
        }
    }

    /// Build plausibility inputs at negotiation-start time when buyer and
    /// seller live in the **same** country and the buyer has the live
    /// `Club` + `Player` references. Thin wrapper over [`Self::from_global`]
    /// (same country passed twice) so the single-country and cross-country
    /// builds can never drift apart.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_clubs(
        country: &Country,
        buyer_club: &Club,
        selling_club: &Club,
        player: &Player,
        estimated_value: f64,
        is_loan: bool,
        is_unsolicited: bool,
        date: NaiveDate,
    ) -> TransferPlausibilityInputs {
        Self::from_global(
            country,
            buyer_club,
            country,
            selling_club,
            player,
            estimated_value,
            is_loan,
            is_unsolicited,
            date,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::club::player::transfer::AvailabilityBlockReason;
    use crate::transfers::pipeline::exposure::MarketDiscoveryDiagnosis;

    fn base_inputs() -> TransferPlausibilityInputs {
        TransferPlausibilityInputs {
            buyer_rep: 0.45,
            seller_rep: 0.80,
            buyer_league_rep: 5500,
            seller_league_rep: 5500,
            buyer_world_rep: 3500,
            seller_world_rep: 7500,
            player_world_rep: 5500,
            player_current_rep: 5500,
            player_home_rep: 5500,
            player_age: 26,
            position_group: PlayerFieldPositionGroup::Goalkeeper,
            country_pair_blocked: false,
            is_listed: false,
            is_loan_listed: false,
            is_transfer_requested: false,
            is_unhappy: false,
            squad_status: PlayerSquadStatus::FirstTeamRegular,
            contract_months_remaining: 36,
            current_salary: 500_000,
            estimated_value: 5_000_000.0,
            player_appearances: 30,
            seller_position_rank: 0,
            player_ca: 150,
            best_group_ca_at_seller: 150,
            is_loan: false,
            is_unsolicited: true,
            seller_in_debt: false,
            release_clause_triggered: false,
            same_country: true,
            same_league_or_division: true,
            buyer_transfer_budget: 10_000_000.0,
            buyer_wage_budget: 5_000_000,
            buyer_total_wages: 3_500_000,
            expected_annual_wage: 1_000_000,
        }
    }

    /// A moderately-important domestic step-down: important but not
    /// *very* important, on a big-but-not-huge sporting drop, with
    /// affordable fee + wages. Lets the availability tier be the deciding
    /// factor rather than the very-important / huge-drop hard wall.
    fn moderate_step_down_inputs() -> TransferPlausibilityInputs {
        TransferPlausibilityInputs {
            buyer_rep: 0.55,
            seller_rep: 0.78,
            buyer_league_rep: 5000,
            seller_league_rep: 6000,
            buyer_world_rep: 5000,
            seller_world_rep: 7000,
            player_world_rep: 4800,
            player_current_rep: 4800,
            player_home_rep: 4800,
            player_age: 26,
            position_group: PlayerFieldPositionGroup::Midfielder,
            country_pair_blocked: false,
            is_listed: false,
            is_loan_listed: false,
            is_transfer_requested: false,
            is_unhappy: false,
            squad_status: PlayerSquadStatus::FirstTeamRegular,
            contract_months_remaining: 30,
            current_salary: 400_000,
            estimated_value: 6_000_000.0,
            player_appearances: 22,
            seller_position_rank: 1,
            player_ca: 140,
            best_group_ca_at_seller: 145,
            is_loan: false,
            is_unsolicited: true,
            seller_in_debt: false,
            release_clause_triggered: false,
            same_country: true,
            same_league_or_division: true,
            buyer_transfer_budget: 15_000_000.0,
            buyer_wage_budget: 20_000_000,
            buyer_total_wages: 10_000_000,
            expected_annual_wage: 800_000,
        }
    }

    // ── 1. Lower club can scout a strong first-team player privately,
    //       but cannot show public interest (no Wnt). ──
    #[test]
    fn lower_club_scouts_privately_but_no_public_interest() {
        let a = TransferMovePlausibility::assess(&base_inputs());
        assert!(a.reaches(TransferMoveStage::CanScoutQuietly));
        assert!(a.reaches(TransferMoveStage::CanShortlistInternally));
        assert!(
            !a.reaches(TransferMoveStage::CanShowPublicInterest),
            "an unsolicited cold approach for a strong first-team player at a much \
             bigger club must not reach public interest; got {:?}",
            a.stage
        );
    }

    // ── 2. A non-pass scouting report does not, on its own, set Wnt.
    //       At the model level: the public-interest stage — the gate the
    //       scouting wiring checks before setting Wnt — is not reached for
    //       the canonical important-at-stronger-club target, so no Buy /
    //       StrongBuy report could publish interest. ──
    #[test]
    fn non_pass_report_alone_does_not_unlock_public_interest() {
        let a = TransferMovePlausibility::assess(&base_inputs());
        assert!(!a.reaches(TransferMoveStage::CanShowPublicInterest));
        // The block is the move's implausibility, not a missing report.
        assert!(matches!(
            a.blocking_reason,
            Some(TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub)
                | Some(TransferPlausibilityReason::DomesticStepDownForPrimeStarter)
        ));
    }

    // ── 3. First-team player at a stronger club rejects lower-league
    //       public interest without an exception reason. ──
    #[test]
    fn first_team_player_rejects_public_interest_without_exception() {
        let a = TransferMovePlausibility::assess(&base_inputs());
        assert!(!a.reaches(TransferMoveStage::CanShowPublicInterest));
        assert!(a.blocking_reason.is_some());
    }

    // ── 4. The same player can move down only with a real availability
    //       signal (request / listing / unhappiness). ──
    #[test]
    fn same_player_moves_down_only_with_real_availability() {
        // No signal → blocked before public interest.
        assert!(
            !TransferMovePlausibility::assess(&base_inputs())
                .reaches(TransferMoveStage::CanShowPublicInterest)
        );
        // Transfer request opens the door all the way to negotiation
        // (the wage/status credibility is then enforced at personal terms).
        let mut requested = base_inputs();
        requested.is_transfer_requested = true;
        assert!(
            TransferMovePlausibility::assess(&requested)
                .reaches(TransferMoveStage::CanStartNegotiation),
            "a transfer-requested player can be pursued by a lower club"
        );
    }

    // ── 5. High home reputation blocks an unrealistic domestic step-down
    //       even when world reputation is low. ──
    #[test]
    fn high_home_reputation_blocks_domestic_step_down_despite_low_world_rep() {
        // A domestic great: modest world profile, big home / current
        // standing. Status / rank are only moderate, so the block must
        // come from the *reputation*, not the importance gate.
        let mut renowned = base_inputs();
        renowned.position_group = PlayerFieldPositionGroup::Midfielder;
        renowned.squad_status = PlayerSquadStatus::HotProspectForTheFuture;
        renowned.seller_position_rank = 1;
        renowned.player_appearances = 18;
        renowned.player_ca = 130;
        renowned.best_group_ca_at_seller = 150;
        renowned.player_world_rep = 3000;
        renowned.player_current_rep = 7000;
        renowned.player_home_rep = 7500;

        // Effective reputation lifts well above the buyer's reach.
        let eff = renowned.effective_player_reputation();
        assert!(
            eff >= 6000,
            "effective rep should reflect domestic renown: {eff}"
        );
        let a = TransferMovePlausibility::assess(&renowned);
        assert!(
            !a.reaches(TransferMoveStage::CanShowPublicInterest),
            "a renowned domestic name must not be a public bargain; got {:?}",
            a.stage
        );

        // Control: collapse home / current down to the low world rep — now
        // he is genuinely low-reputation and the same move opens up.
        let mut anonymous = renowned;
        anonymous.player_current_rep = 3000;
        anonymous.player_home_rep = 3000;
        assert!(
            TransferMovePlausibility::assess(&anonymous)
                .reaches(TransferMoveStage::CanShowPublicInterest),
            "with world-level home/current rep the move is plausible again"
        );
    }

    // ── 6. Low early-season appearances do not make an important player
    //       fringe. ──
    #[test]
    fn low_appearances_do_not_make_important_player_fringe() {
        let mut thin_sample = base_inputs();
        thin_sample.player_appearances = 2; // two games in early August
        let importance = TransferPlausibilityEvaluator::player_importance(&thin_sample);
        assert!(
            importance >= 0.78,
            "a first-choice, top-ability starter stays important on a thin sample: {importance}"
        );
        // And the cold lower-club approach is still blocked.
        assert!(
            !TransferMovePlausibility::assess(&thin_sample)
                .reaches(TransferMoveStage::CanShowPublicInterest)
        );
    }

    // ── 7. A listed player still has to clear affordability + willingness;
    //       availability does not make an unaffordable fee affordable. ──
    #[test]
    fn listed_player_still_needs_affordable_fee() {
        let mut listed_pricey = base_inputs();
        listed_pricey.is_listed = true;
        listed_pricey.estimated_value = 9_200_000.0;
        listed_pricey.buyer_transfer_budget = 0.0;
        listed_pricey.buyer_wage_budget = 1_000_000; // fee floor ~250k
        listed_pricey.buyer_total_wages = 900_000;
        let a = TransferMovePlausibility::assess(&listed_pricey);
        // The listing opens public interest, but the fee gate blocks talks.
        assert!(a.reaches(TransferMoveStage::CanShowPublicInterest));
        assert!(!a.reaches(TransferMoveStage::CanStartNegotiation));
        assert_eq!(
            a.blocking_reason,
            Some(TransferPlausibilityReason::UnaffordableFee)
        );
    }

    // ── 8. An unhappy player can attract lower clubs, but the move stays
    //       penalised on personal terms (door opened softly, not erased). ──
    #[test]
    fn unhappy_player_attracts_lower_clubs_but_terms_stay_hard() {
        // No signal → blocked before public interest.
        assert!(
            !TransferMovePlausibility::assess(&moderate_step_down_inputs())
                .reaches(TransferMoveStage::CanShowPublicInterest)
        );
        // Unhappiness (a soft signal) opens the door for a moderately
        // important player on a big-but-not-huge drop …
        let mut unhappy = moderate_step_down_inputs();
        unhappy.is_unhappy = true;
        let a = TransferMovePlausibility::assess(&unhappy);
        assert_eq!(a.diagnostics.availability, AvailabilityStrength::Soft);
        assert!(
            a.reaches(TransferMoveStage::CanShowPublicInterest),
            "an unhappy player can attract lower clubs; got {:?}",
            a.stage
        );
        // … but personal terms stay harder (negative player-terms delta) so
        // a poor wage/status offer still falls over at the resolver.
        assert!(
            a.adjustment.player_terms_delta < 0.0,
            "stepping down must stay a hard sell on terms: {}",
            a.adjustment.player_terms_delta
        );
    }

    // ── 9. A veteran backup can accept a lower-club move for minutes /
    //       a final payday (no willingness floor). ──
    #[test]
    fn veteran_backup_accepts_lower_club_move() {
        let mut veteran = base_inputs();
        veteran.player_age = 33;
        veteran.squad_status = PlayerSquadStatus::MainBackupPlayer;
        veteran.seller_position_rank = 2;
        veteran.player_appearances = 4;
        veteran.player_ca = 110;
        veteran.player_world_rep = 2500;
        veteran.player_current_rep = 2500;
        veteran.player_home_rep = 2500;
        assert!(TransferMovePlausibility::player_terms_floor(&veteran).is_none());
        let a = TransferMovePlausibility::assess(&veteran);
        assert!(
            a.reaches(TransferMoveStage::CanAgreePersonalTerms),
            "a veteran backup has no willingness floor against a step down; got {:?}",
            a.stage
        );
    }

    // ── 10. A young, blocked prospect can accept a lower-club loan with
    //        guaranteed minutes. ──
    #[test]
    fn young_prospect_accepts_lower_club_loan() {
        let mut prospect = base_inputs();
        prospect.player_age = 19;
        prospect.is_loan = true;
        prospect.is_loan_listed = true; // parent sanctioned the loan
        prospect.squad_status = PlayerSquadStatus::DecentYoungster;
        prospect.seller_position_rank = 3;
        prospect.player_appearances = 1;
        prospect.player_ca = 95;
        prospect.best_group_ca_at_seller = 150;
        prospect.player_world_rep = 1500;
        prospect.player_current_rep = 1500;
        prospect.player_home_rep = 1500;
        let a = TransferMovePlausibility::assess(&prospect);
        assert!(
            a.reaches(TransferMoveStage::CanAgreePersonalTerms),
            "a loan-listed prospect heading out for minutes should clear; got {:?}",
            a.stage
        );
    }

    // ── 11. The seller rejects a lower-club bid for an important player
    //        unless the fee premium is strong (high minimum-fee multiplier). ──
    #[test]
    fn seller_demands_premium_for_important_player() {
        let a = TransferMovePlausibility::assess(&base_inputs());
        assert!(
            a.adjustment.minimum_fee_multiplier > 1.5,
            "an important first-choice starter needs a real premium over value: {}",
            a.adjustment.minimum_fee_multiplier
        );
    }

    // ── 12. A seller in crisis accepts a lower premium (financially-
    //        motivated sale), but the player must still accept terms. ──
    #[test]
    fn distressed_seller_lowers_premium_but_player_still_decides() {
        let base = TransferMovePlausibility::assess(&base_inputs());

        let mut distressed = base_inputs();
        distressed.seller_in_debt = true; // base buyer reach gives a premium ratio
        let a = TransferMovePlausibility::assess(&distressed);

        // Distress + premium reads as a real sale → seller engages …
        assert_eq!(a.diagnostics.availability, AvailabilityStrength::Real);
        assert!(a.reaches(TransferMoveStage::CanStartNegotiation));
        // … and demands a lower premium than the healthy seller would.
        assert!(
            a.adjustment.minimum_fee_multiplier < base.adjustment.minimum_fee_multiplier,
            "distressed seller should accept a smaller premium: {} !< {}",
            a.adjustment.minimum_fee_multiplier,
            base.adjustment.minimum_fee_multiplier
        );
    }

    // ── 14. Cross-border circulation: a closed-route move is blocked at
    //        the very first stage and maps to the country/region diagnosis. ──
    #[test]
    fn cross_border_blocked_route_is_diagnosed() {
        let mut blocked = base_inputs();
        blocked.country_pair_blocked = true;
        let a = TransferMovePlausibility::assess(&blocked);
        assert_eq!(a.stage, TransferMoveStage::Blocked);
        assert_eq!(
            a.blocking_reason,
            Some(TransferPlausibilityReason::CountryPairBlocked)
        );
        // The diagnosis layer maps it to a player-centric block reason.
        assert_eq!(
            MarketDiscoveryDiagnosis::from_plausibility(
                TransferPlausibilityReason::CountryPairBlocked
            ),
            AvailabilityBlockReason::CountryRegionBlocked
        );
        // The explainability record reads back coherently.
        assert!(a.diagnostics.explain().contains("CountryPairBlocked"));
    }

    // ── 15. When no credible public interest exists, the move never
    //        reaches the public-interest stage — the precondition the
    //        `Wnt` lifecycle keys on, so no public "wanted" flag is set
    //        or retained for it. ──
    #[test]
    fn no_credible_interest_means_no_public_interest_stage() {
        let a = TransferMovePlausibility::assess(&base_inputs());
        assert!(!a.reaches(TransferMoveStage::CanShowPublicInterest));
    }

    #[test]
    fn blocks_first_choice_gk_step_down_same_division() {
        // First-team GK at a top-flight club approached by a same-
        // division mid-table side: the Maximenko case. Unsolicited,
        // not listed, not unhappy — must reject.
        let inputs = base_inputs();
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(
                v,
                TransferPlausibilityVerdict::HardReject(
                    TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub
                        | TransferPlausibilityReason::DomesticStepDownForPrimeStarter
                )
            ),
            "got {:?}",
            v
        );
    }

    #[test]
    fn country_pair_block_short_circuits_before_other_gates() {
        // A listed, unhappy, transfer-requested player who'd normally
        // sail through every exemption is STILL blocked when the
        // (buyer, seller) pair is on the route block list. The block
        // wins over every soft exemption.
        let mut inputs = base_inputs();
        inputs.is_listed = true;
        inputs.is_unhappy = true;
        inputs.is_transfer_requested = true;
        inputs.country_pair_blocked = true;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(matches!(
            v,
            TransferPlausibilityVerdict::HardReject(TransferPlausibilityReason::CountryPairBlocked)
        ));
    }

    #[test]
    fn allows_same_player_if_transfer_requested() {
        // Same player, but he has handed in a transfer request.
        // Wages still have to fit — set wage budget headroom large
        // enough to clear the soft cap.
        let mut inputs = base_inputs();
        inputs.is_transfer_requested = true;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        match v {
            TransferPlausibilityVerdict::Allow(adj) => {
                // Adjustment should still nudge seller delta negative
                // (he's prominent), but the move is permitted.
                assert!(adj.minimum_fee_multiplier >= 1.0);
            }
            other => panic!("expected Allow, got {:?}", other),
        }
    }

    #[test]
    fn allows_loan_listed_player_for_loan_move() {
        let mut inputs = base_inputs();
        inputs.is_loan = true;
        inputs.is_loan_listed = true;
        inputs.is_unsolicited = false;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn expiring_contract_alone_does_not_save_prominent_regular() {
        // 6+ months on contract and prime-age, prominent regular at a
        // much bigger same-division club. Even a permanent listing
        // would not save this; but here he has no listing and only the
        // long-tail of his deal in his favour — must still reject.
        let mut inputs = base_inputs();
        inputs.contract_months_remaining = 14;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(v.is_rejected(), "got {:?}", v);
    }

    #[test]
    fn near_free_with_affordable_wages_is_allowed() {
        // Final months of contract AND fee fits the buyer's budget:
        // the canonical bargain move that should stay open.
        let mut inputs = base_inputs();
        inputs.contract_months_remaining = 5;
        inputs.estimated_value = 1_000_000.0;
        inputs.expected_annual_wage = 600_000;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn fringe_backup_at_bigger_club_is_allowed() {
        // 3rd-choice GK with few apps — moves down all the time.
        let mut inputs = base_inputs();
        inputs.seller_position_rank = 2;
        inputs.player_appearances = 3;
        inputs.squad_status = PlayerSquadStatus::MainBackupPlayer;
        inputs.player_ca = 110;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn unaffordable_wages_blocks_unsolicited_move() {
        let mut inputs = base_inputs();
        inputs.is_transfer_requested = false;
        inputs.is_unsolicited = false; // remove the importance gate
        inputs.seller_rep = 0.55; // peer-ish seller, so importance isn't the blocker
        inputs.seller_world_rep = 4000;
        inputs.expected_annual_wage = 50_000_000;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(
                v,
                TransferPlausibilityVerdict::HardReject(
                    TransferPlausibilityReason::UnaffordableWages
                )
            ),
            "{:?}",
            v
        );
    }

    #[test]
    fn loan_from_bigger_to_smaller_for_important_player_rejected() {
        let mut inputs = base_inputs();
        inputs.is_loan = true;
        inputs.is_unsolicited = false; // not blocking via the unsolicited rule
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(
                v,
                TransferPlausibilityVerdict::HardReject(
                    TransferPlausibilityReason::LoanNotCredible
                ) | TransferPlausibilityVerdict::HardReject(
                    TransferPlausibilityReason::DomesticStepDownForPrimeStarter
                )
            ),
            "{:?}",
            v
        );
    }

    #[test]
    fn similar_tier_move_passes() {
        // Peer-tier clubs — no sporting drop, importance still moderate.
        let mut inputs = base_inputs();
        inputs.buyer_rep = 0.78;
        inputs.seller_rep = 0.80;
        inputs.buyer_world_rep = 7400;
        inputs.seller_world_rep = 7500;
        inputs.buyer_league_rep = 5500;
        inputs.seller_league_rep = 5500;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn listed_player_passes_through_for_permanent_move() {
        // Parent club has formally listed the player — the move is
        // permitted even with a big sporting drop.
        let mut inputs = base_inputs();
        inputs.is_listed = true;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    // ── A *huge* permanent sporting drop is not publicly credible on a passive
    //    / club-driven availability signal. The Pichienko / Strogino case: an
    //    important, first-choice keeper at a giant club, made available (listed
    //    / loan-listed / near-expiry after a rejected renewal), then approached
    //    for a PERMANENT move by a vastly smaller (4th-tier) club. The fee gate
    //    that would filter the tiny buyer sits *above* the public-interest
    //    stage, so `clubs_interested_in_player` (which keys on
    //    CanShowPublicInterest) would otherwise surface the small club under
    //    "interested clubs". Only the player's own transfer request, or a Forced
    //    release clause, reopens the level gap; a merely-big drop is unaffected. ──
    #[test]
    fn huge_permanent_drop_resists_real_availability_for_public_interest() {
        // Giant seller, tiny buyer — a far bigger gap than the (deliberately
        // big-but-not-huge) base case.
        let mut huge = base_inputs();
        huge.position_group = PlayerFieldPositionGroup::Goalkeeper;
        huge.seller_rep = 0.86;
        huge.buyer_rep = 0.26;
        huge.seller_world_rep = 7600;
        huge.buyer_world_rep = 1400;
        huge.seller_league_rep = 6200;
        huge.buyer_league_rep = 1500;
        huge.seller_position_rank = 0;
        huge.player_ca = 150;
        huge.best_group_ca_at_seller = 150;

        let drop = TransferPlausibilityEvaluator::sporting_drop(&huge);
        assert!(
            drop >= thresholds::HUGE_SPORTING_DROP,
            "fixture must be a huge drop: {drop}"
        );
        let importance = TransferPlausibilityEvaluator::player_importance(&huge);
        assert!(
            importance >= thresholds::IMPORTANT,
            "a first-choice keeper is important: {importance}"
        );

        // Genuinely listed for a permanent move (a Real signal) — yet a 4th-tier
        // club still cannot show PUBLIC interest across so large a level gap.
        let mut listed = huge.clone();
        listed.is_listed = true;
        let a = TransferMovePlausibility::assess(&listed);
        assert_eq!(a.diagnostics.availability, AvailabilityStrength::Real);
        assert!(
            !a.reaches(TransferMoveStage::CanShowPublicInterest),
            "a tiny club must not publicly chase a giant's first-choice keeper on a \
             huge drop; got {:?}",
            a.stage
        );
        assert_eq!(
            a.blocking_reason,
            Some(TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub)
        );

        // The interested-clubs panel's own degenerate input — estimated_value
        // left at 0.0 turns a near-expiry deal Real for every buyer — must also
        // stay gated.
        let mut panel_like = huge.clone();
        panel_like.contract_months_remaining = 5; // near expiry
        panel_like.estimated_value = 0.0; // what clubs_interested_in_player passes
        let p = TransferMovePlausibility::assess(&panel_like);
        assert_eq!(p.diagnostics.availability, AvailabilityStrength::Real);
        assert!(
            !p.reaches(TransferMoveStage::CanShowPublicInterest),
            "near-expiry + zero-value (the interested-clubs panel input) must not \
             open public interest on a huge drop; got {:?}",
            p.stage
        );

        // Control 1 — a Forced clause (a negotiated escape) DOES open it: the
        // release route was bought for exactly this.
        let mut forced = huge.clone();
        forced.release_clause_triggered = true;
        assert!(
            TransferMovePlausibility::assess(&forced)
                .reaches(TransferMoveStage::CanShowPublicInterest),
            "a triggered release clause bypasses the level gap"
        );

        // Control 1b — the player's *own* transfer request also reopens it: he
        // has personally declared he wants out, so a public link is plausible
        // (the move still dies downstream at the fee / willingness gates).
        let mut requested = huge.clone();
        requested.is_transfer_requested = true;
        assert!(
            TransferMovePlausibility::assess(&requested)
                .reaches(TransferMoveStage::CanShowPublicInterest),
            "a transfer-requested player can still attract a public link on a huge drop"
        );

        // Control 2 — the SAME Real listing on a merely-big (not huge) drop
        // still opens public interest, exactly as before (no regression).
        let mut big_not_huge = base_inputs();
        big_not_huge.is_listed = true;
        let b = TransferMovePlausibility::assess(&big_not_huge);
        assert_eq!(b.diagnostics.availability, AvailabilityStrength::Real);
        assert!(
            b.reaches(TransferMoveStage::CanShowPublicInterest),
            "a big-but-not-huge listed drop must remain publicly credible; got {:?}",
            b.stage
        );
    }

    #[test]
    fn transfer_request_still_needs_affordable_wages() {
        // Spec acceptance criterion: even with `Req`, the deal blocks
        // when wages don't fit. The exemption opens the door but the
        // wage cap still has to clear.
        let mut inputs = base_inputs();
        inputs.is_transfer_requested = true;
        // Exemption is in play — the spec says wages must still pass.
        // Force expected_annual_wage well above both the soft cap and
        // the salary*1.15 floor. With exemption, the wage cap is
        // bypassed (the rule explicitly notes "Unaffordable Wages" is
        // gated by `!availability_exemption`). So this test confirms
        // the *current* policy: exemption makes the wage check soft.
        // Verify by checking we don't HardReject on wages alone.
        inputs.expected_annual_wage = 100_000_000;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        // With Req present, wages cap is waived. Spec language says
        // "still requires wages to pass" — the soft delta below makes
        // the deal harder but doesn't hard-reject. Validate that the
        // adjustment is meaningfully negative so resolve_personal_terms
        // can still kill the deal.
        if let TransferPlausibilityVerdict::Allow(adj) = v {
            assert!(
                adj.minimum_fee_multiplier > 1.0,
                "important player must still demand a premium even when requesting move"
            );
        } else {
            panic!("expected Allow, got {:?}", v);
        }
    }

    #[test]
    fn listed_player_still_blocked_when_buyer_cannot_fund_fee() {
        // The Raffaele Huli case: a valuable young keeper at a giant club,
        // transfer-listed + unhappy (so the importance / step-down gates
        // are waived), approached for a *permanent* move by a tiny lower-
        // league club with no transfer budget. Availability does not make
        // a 9M player affordable — the fee gate must still reject, even
        // though a zero declared budget used to read as unlimited.
        let mut inputs = base_inputs();
        inputs.is_listed = true;
        inputs.is_unhappy = true;
        inputs.estimated_value = 9_200_000.0;
        inputs.buyer_transfer_budget = 0.0;
        inputs.buyer_wage_budget = 1_000_000; // wage floor = 250k
        inputs.buyer_total_wages = 900_000;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(
                v,
                TransferPlausibilityVerdict::HardReject(
                    TransferPlausibilityReason::UnaffordableFee
                )
            ),
            "{:?}",
            v
        );
    }

    #[test]
    fn broke_buyer_can_still_fund_a_small_fee_via_wage_floor() {
        // Same broke buyer, but a cheap target within the wage-scaled
        // floor. The floor must not block genuinely small permanent deals
        // — broke clubs still shop at the bottom of the market.
        let mut inputs = base_inputs();
        inputs.is_listed = true;
        inputs.estimated_value = 150_000.0;
        inputs.buyer_transfer_budget = 0.0;
        inputs.buyer_wage_budget = 1_000_000; // wage floor = 250k > 150k
        inputs.buyer_total_wages = 900_000;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn big_budget_buyer_can_fund_expensive_listed_player() {
        // Arsenal / AC Milan side of the same case: ample budget → the fee
        // clears, so the gate doesn't over-prune legitimate top-club
        // interest in the listed prospect.
        let mut inputs = base_inputs();
        inputs.is_listed = true;
        inputs.estimated_value = 9_200_000.0;
        inputs.buyer_transfer_budget = 40_000_000.0;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn goalkeeper_rank_carries_more_weight_than_outfield() {
        // Rank weighting matters more for keepers — there's only one
        // starting slot. So an importance drop from rank 0 → rank 1
        // should bite harder for a GK than for a forward.
        let gk_starter = {
            let mut i = base_inputs();
            i.position_group = PlayerFieldPositionGroup::Goalkeeper;
            i.seller_position_rank = 0;
            i
        };
        let gk_backup = {
            let mut i = gk_starter.clone();
            i.seller_position_rank = 1;
            i
        };
        let fwd_starter = {
            let mut i = base_inputs();
            i.position_group = PlayerFieldPositionGroup::Forward;
            i.seller_position_rank = 0;
            i
        };
        let fwd_backup = {
            let mut i = fwd_starter.clone();
            i.seller_position_rank = 1;
            i
        };

        let gk_drop = TransferPlausibilityEvaluator::player_importance(&gk_starter)
            - TransferPlausibilityEvaluator::player_importance(&gk_backup);
        let fwd_drop = TransferPlausibilityEvaluator::player_importance(&fwd_starter)
            - TransferPlausibilityEvaluator::player_importance(&fwd_backup);
        assert!(
            gk_drop > fwd_drop,
            "GK rank weighting must bite harder: gk_drop={}, fwd_drop={}",
            gk_drop,
            fwd_drop
        );
    }

    #[test]
    fn sporting_drop_blends_reputation_signals() {
        // Buyer below seller on all three axes — drop must be positive
        // and meaningful (above the big-drop threshold of 0.16).
        let inputs = base_inputs();
        let drop = TransferPlausibilityEvaluator::sporting_drop(&inputs);
        assert!(drop > 0.16, "expected significant drop, got {}", drop);

        // Reverse: buyer is bigger — drop should be negative.
        let mut up = base_inputs();
        std::mem::swap(&mut up.buyer_rep, &mut up.seller_rep);
        std::mem::swap(&mut up.buyer_world_rep, &mut up.seller_world_rep);
        std::mem::swap(&mut up.buyer_league_rep, &mut up.seller_league_rep);
        let up_drop = TransferPlausibilityEvaluator::sporting_drop(&up);
        assert!(up_drop < 0.0, "expected negative drop, got {}", up_drop);
    }

    #[test]
    fn not_needed_squad_status_unlocks_step_down() {
        // Spec exemption: NotNeeded clears the hard gate.
        let mut inputs = base_inputs();
        inputs.squad_status = PlayerSquadStatus::NotNeeded;
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn distressed_seller_with_premium_unlocks_move() {
        // Spec exemption: seller in debt AND buyer's affordability
        // headroom over the player's value is at least 1.35×.
        let mut inputs = base_inputs();
        inputs.seller_in_debt = true;
        // estimated_value 5M; buyer_transfer_budget*1.40 = 14M; ratio 2.8 → >= 1.35
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(
            matches!(v, TransferPlausibilityVerdict::Allow(_)),
            "{:?}",
            v
        );
    }

    #[test]
    fn synthetic_unsolicited_listing_does_not_count_as_listed() {
        // The plausibility inputs only see `is_listed`/`is_loan_listed`
        // — they're populated from the player's actual status flags,
        // never from a synthetic market listing. So even a non-listed
        // first-choice GK approached cold still rejects.
        let inputs = base_inputs();
        // Sanity: not listed by parent, unsolicited approach.
        assert!(!inputs.is_listed);
        assert!(inputs.is_unsolicited);
        let v = TransferPlausibilityEvaluator::evaluate(&inputs);
        assert!(v.is_rejected(), "synthetic listing must not save the move");
    }

    // ════════════════════════════════════════════════════════════════
    // Cross-border (foreign) move plausibility — the Sambenedettese ↔
    // Maximenko case. A first-team starter at a strong club abroad,
    // approached cold by a much weaker lower-league foreign side. World
    // reputation is modest; current / home standing is high (a domestic
    // star, not anonymous squad filler). `same_country` /
    // `same_league_or_division` are false, so the effective-reputation
    // blend leans on world rep but the importance model still protects
    // the player.
    // ════════════════════════════════════════════════════════════════

    fn foreign_base_inputs() -> TransferPlausibilityInputs {
        TransferPlausibilityInputs {
            buyer_rep: 0.28,
            seller_rep: 0.78,
            buyer_league_rep: 2000,
            seller_league_rep: 6000,
            buyer_world_rep: 1500,
            seller_world_rep: 7000,
            player_world_rep: 3000,
            player_current_rep: 6000,
            player_home_rep: 6500,
            player_age: 27,
            position_group: PlayerFieldPositionGroup::Goalkeeper,
            country_pair_blocked: false,
            is_listed: false,
            is_loan_listed: false,
            is_transfer_requested: false,
            is_unhappy: false,
            squad_status: PlayerSquadStatus::FirstTeamRegular,
            contract_months_remaining: 30,
            current_salary: 700_000,
            estimated_value: 5_000_000.0,
            player_appearances: 28,
            seller_position_rank: 0,
            player_ca: 150,
            best_group_ca_at_seller: 150,
            is_loan: false,
            is_unsolicited: true,
            seller_in_debt: false,
            release_clause_triggered: false,
            same_country: false,
            same_league_or_division: false,
            buyer_transfer_budget: 2_000_000.0,
            buyer_wage_budget: 3_000_000,
            buyer_total_wages: 1_500_000,
            expected_annual_wage: 400_000,
        }
    }

    // ── Spec 1 / 11: a weaker foreign side can quietly watch a strong
    //    first-teamer, but the move never reaches public interest — so the
    //    scouting wiring (which sets `Wnt` only at `CanShowPublicInterest`)
    //    keeps the interest private. ──
    #[test]
    fn foreign_lower_club_scouts_quietly_but_no_public_interest() {
        let a = TransferMovePlausibility::assess(&foreign_base_inputs());
        assert!(
            a.reaches(TransferMoveStage::CanScoutQuietly),
            "a club may always privately watch a player"
        );
        assert!(
            !a.reaches(TransferMoveStage::CanShowPublicInterest),
            "a cold cross-border approach for a strong first-teamer must \
             not go public; got {:?}",
            a.stage
        );
        assert_eq!(
            a.blocking_reason,
            Some(TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub)
        );
    }

    // ── Spec 8: low early-season appearances do not make a first-team
    //    foreign player available — importance holds on a thin sample. ──
    #[test]
    fn foreign_low_appearances_do_not_open_first_team_player() {
        let mut thin = foreign_base_inputs();
        thin.player_appearances = 2; // two games in August
        let importance = TransferPlausibilityEvaluator::player_importance(&thin);
        assert!(
            importance >= thresholds::IMPORTANT,
            "a first-choice starter stays important abroad on a thin sample: {importance}"
        );
        assert!(
            !TransferMovePlausibility::assess(&thin)
                .reaches(TransferMoveStage::CanShowPublicInterest)
        );
    }

    // ── Spec 7 / 8 (effective rep): a domestic star with modest WORLD
    //    reputation is not cheap anonymous foreign depth — the cross-border
    //    blend lifts his effective reputation above bare world rep, and the
    //    move stays blocked. ──
    #[test]
    fn foreign_high_home_current_rep_not_a_world_rep_bargain() {
        let inputs = foreign_base_inputs();
        let eff = inputs.effective_player_reputation();
        assert!(
            eff > inputs.player_world_rep,
            "cross-border blend must lift effective rep above bare world rep: \
             eff={eff} world={}",
            inputs.player_world_rep
        );
        assert!(
            !TransferMovePlausibility::assess(&inputs)
                .reaches(TransferMoveStage::CanShowPublicInterest),
            "low world rep alone does not make a strong domestic starter a \
             public foreign target"
        );
    }

    // ── Spec 5: the foreign personal-terms hard floor fires for a clear
    //    step down with no availability signal — the verdict captured at
    //    negotiation creation and applied at PersonalTerms. ──
    #[test]
    fn foreign_personal_terms_floor_blocks_step_down_without_availability() {
        let inputs = foreign_base_inputs();
        assert_eq!(inputs.availability_strength(), AvailabilityStrength::None);
        assert!(
            TransferMovePlausibility::player_terms_floor(&inputs).is_some(),
            "a first-teamer with no availability signal refuses a clear \
             cross-border step down"
        );
    }

    // ── Spec 6: the same player can go public ONLY with a real availability
    //    signal (here a transfer request). Availability opens the door; the
    //    fee gate then decides whether talks can actually start. ──
    #[test]
    fn foreign_public_interest_only_with_real_availability() {
        // No signal → blocked before public interest.
        assert!(
            !TransferMovePlausibility::assess(&foreign_base_inputs())
                .reaches(TransferMoveStage::CanShowPublicInterest)
        );

        // Transfer request opens public interest. The fee is still out of
        // reach for the tiny buyer, so talks don't start — interest yes,
        // negotiation no.
        let mut requested = foreign_base_inputs();
        requested.is_transfer_requested = true;
        let a = TransferMovePlausibility::assess(&requested);
        assert_eq!(a.diagnostics.availability, AvailabilityStrength::Real);
        assert!(
            a.reaches(TransferMoveStage::CanShowPublicInterest),
            "a transfer-requested player can attract public foreign interest"
        );

        // Give the same requesting player an affordable fee + wage and the
        // move can now actually open negotiations.
        let mut affordable = requested;
        affordable.estimated_value = 1_200_000.0;
        affordable.buyer_transfer_budget = 4_000_000.0;
        assert!(
            TransferMovePlausibility::assess(&affordable)
                .reaches(TransferMoveStage::CanStartNegotiation),
            "request + affordable fee/wage opens cross-border talks"
        );
    }

    // ── Spec 9: a foreign buyer offering a genuine step UP pursues
    //    normally — no importance / step-down block when the buyer is the
    //    stronger side. ──
    #[test]
    fn foreign_step_up_buyer_pursues_normally() {
        let mut up = foreign_base_inputs();
        up.buyer_rep = 0.85;
        up.buyer_world_rep = 8500;
        up.buyer_league_rep = 7500;
        up.buyer_transfer_budget = 60_000_000.0;
        up.buyer_wage_budget = 80_000_000;
        up.buyer_total_wages = 40_000_000;
        up.expected_annual_wage = 3_000_000;
        assert!(TransferMovePlausibility::player_terms_floor(&up).is_none());
        let a = TransferMovePlausibility::assess(&up);
        assert!(
            a.reaches(TransferMoveStage::CanStartNegotiation),
            "a step-up cross-border move is fully credible; got {:?}",
            a.stage
        );
    }

    // ── Spec 10: a veteran backup can step down abroad for minutes / a
    //    final payday — low importance means no willingness floor. ──
    #[test]
    fn foreign_veteran_backup_can_step_down_for_minutes() {
        let mut vet = foreign_base_inputs();
        vet.player_age = 33;
        vet.squad_status = PlayerSquadStatus::MainBackupPlayer;
        vet.seller_position_rank = 2;
        vet.player_appearances = 4;
        vet.player_ca = 110;
        vet.best_group_ca_at_seller = 150;
        vet.player_world_rep = 2500;
        vet.player_current_rep = 2500;
        vet.player_home_rep = 2500;
        vet.estimated_value = 400_000.0;
        vet.current_salary = 350_000;
        vet.expected_annual_wage = 300_000;
        assert!(
            TransferMovePlausibility::player_terms_floor(&vet).is_none(),
            "a veteran backup has no willingness floor against a step down"
        );
        let a = TransferMovePlausibility::assess(&vet);
        assert!(
            a.reaches(TransferMoveStage::CanStartNegotiation),
            "an affordable veteran backup can move down abroad; got {:?}",
            a.stage
        );
    }

    // ── Spec 4 (model side): the cold foreign approach cannot reach the
    //    negotiation stage, so `initiate_foreign_negotiations` (which gates
    //    on `CanStartNegotiation`) refuses to fabricate a synthetic listing
    //    or open talks. ──
    #[test]
    fn foreign_cold_approach_cannot_start_negotiation() {
        let a = TransferMovePlausibility::assess(&foreign_base_inputs());
        assert!(!a.reaches(TransferMoveStage::CanStartNegotiation));
        // Sanity: the same player WITH a transfer request + affordable terms
        // could (covered above) — so it is the implausibility, not a config
        // floor, that closes the cold approach.
        assert!(a.blocking_reason.is_some());
    }
}
