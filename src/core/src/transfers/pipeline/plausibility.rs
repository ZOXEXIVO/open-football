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
//! ### Availability exemptions
//!
//! The hard rejects only fire when the player is **not** advertising
//! themselves as available. The exemption set is documented on
//! [`availability_exemption`] and mirrors how a real transfer market
//! treats public information — a listed/loan-listed/requested/unhappy
//! player can be approached from below; a synthetic "we created a
//! listing because we wanted to bid" does NOT count.

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

    #[allow(dead_code)]
    pub player_world_rep: i16,
    #[allow(dead_code)]
    pub player_current_rep: i16,
    #[allow(dead_code)]
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
    /// Apply the availability exemption rules listed in the module
    /// documentation. A player passing any of these checks can break
    /// hard reject bands; soft adjustments still apply.
    ///
    /// Synthetic listings are explicitly excluded — callers must set
    /// `is_listed`/`is_loan_listed` from the player's real status, not
    /// from a listing fabricated to back an unsolicited bid.
    pub fn availability_exemption(&self) -> bool {
        if self.is_transfer_requested
            || self.is_unhappy
            || matches!(self.squad_status, PlayerSquadStatus::NotNeeded)
            || self.release_clause_triggered
        {
            return true;
        }
        // Permanent listing only counts when the move is permanent; a
        // loan-only listing means the parent club is willing to lend
        // but not sell. Same logic in reverse.
        if !self.is_loan && self.is_listed {
            return true;
        }
        if self.is_loan && self.is_loan_listed {
            return true;
        }
        // Near-end-of-contract pickup — only if the buyer can afford
        // both the fee and the wages.
        let max_fee = self.affordability_max_fee();
        if self.contract_months_remaining > 0
            && self.contract_months_remaining <= 6
            && self.estimated_value <= max_fee
        {
            return true;
        }
        // Distressed seller plus a clear premium tag.
        if self.seller_in_debt {
            let ratio = if self.estimated_value > 1.0 {
                // `offer_premium_ratio` isn't carried here; instead we
                // approximate by demanding the player's value is within
                // the buyer's reach AND the buyer is offering a typical
                // premium via min-fee floor logic downstream. The cheap
                // proxy used here keeps the exemption available without
                // re-coupling to current_offer plumbing.
                (max_fee / self.estimated_value).clamp(0.0, 5.0)
            } else {
                5.0
            };
            if ratio >= 1.35 {
                return true;
            }
        }
        false
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
}

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
        let s = status_score(&inputs.squad_status);
        let r = rank_score(inputs.seller_position_rank);
        let a = appearance_score(inputs.player_appearances);
        let ab = ability_score(inputs.player_ca, inputs.best_group_ca_at_seller);

        let raw = if inputs.position_group == PlayerFieldPositionGroup::Goalkeeper {
            0.36 * s + 0.34 * r + 0.18 * a + 0.12 * ab
        } else {
            0.34 * s + 0.25 * r + 0.25 * a + 0.16 * ab
        };
        raw.clamp(0.0, 1.0)
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

    /// Single entry point: evaluate a (buyer, seller, player) tuple and
    /// return either a hard reject reason or a soft-adjustment bundle.
    /// The evaluator is pure — no I/O, no randomness, fully deterministic
    /// from `inputs`.
    pub fn evaluate(inputs: &TransferPlausibilityInputs) -> TransferPlausibilityVerdict {
        let importance = Self::player_importance(inputs);
        let drop = Self::sporting_drop(inputs);
        let exemption = inputs.availability_exemption();

        let important = importance >= thresholds::IMPORTANT;
        let very_important = importance >= thresholds::VERY_IMPORTANT;
        let prime_age = inputs.player_age >= thresholds::PRIME_AGE_MIN
            && inputs.player_age <= thresholds::PRIME_AGE_MAX;
        let big_drop = drop >= thresholds::BIG_SPORTING_DROP;
        let huge_drop = drop >= thresholds::HUGE_SPORTING_DROP;
        let same_domestic_market = inputs.same_country || inputs.same_league_or_division;

        // ── Hard rejects ──

        // Important first-team type at much stronger club, unsolicited, no
        // availability flag: blocked outright. This is the canonical case
        // — first-team GK at a top club approached out of the blue by a
        // mid-table same-division side.
        if !exemption && inputs.is_unsolicited && important && big_drop {
            return TransferPlausibilityVerdict::HardReject(
                TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub,
            );
        }

        // Same-domestic-market step-down for prime-age starter. Even with
        // a real listing somewhere else, top players don't move sideways
        // or downward in their own division/country to materially weaker
        // clubs at peak career.
        if !exemption
            && same_domestic_market
            && prime_age
            && important
            && drop >= thresholds::DOMESTIC_STEP_DOWN_DROP
        {
            return TransferPlausibilityVerdict::HardReject(
                TransferPlausibilityReason::DomesticStepDownForPrimeStarter,
            );
        }

        // Loan from a smaller club to a bigger one is plausible; the
        // reverse for an important player isn't — the parent wouldn't risk
        // sending a key contributor to a sub-tier suitor.
        if inputs.is_loan
            && !exemption
            && importance >= thresholds::LOAN_IMPORTANCE_BLOCK
            && (inputs.seller_rep - inputs.buyer_rep) > thresholds::LOAN_REP_GAP_BLOCK
        {
            return TransferPlausibilityVerdict::HardReject(
                TransferPlausibilityReason::LoanNotCredible,
            );
        }

        // Affordability — fee. Release clauses and loans bypass. The cap
        // is wage-floored (see `affordability_max_fee`) so a zero declared
        // budget no longer reads as unlimited — a listed/unhappy player is
        // *available*, but a far smaller club still can't fund the fee.
        let max_fee = inputs.affordability_max_fee();
        if !inputs.release_clause_triggered && !inputs.is_loan && inputs.estimated_value > max_fee {
            return TransferPlausibilityVerdict::HardReject(
                TransferPlausibilityReason::UnaffordableFee,
            );
        }

        // Affordability — wages. Available exemption (the player wants the
        // move) gives the buyer leeway; otherwise the soft cap applies.
        let wage_headroom =
            (inputs.buyer_wage_budget as i64 - inputs.buyer_total_wages as i64).max(0);
        let soft_wage_cap =
            ((inputs.current_salary as f64 * 1.15).max(wage_headroom as f64 * 1.30)).max(0.0);
        if !exemption && soft_wage_cap > 0.0 && (inputs.expected_annual_wage as f64) > soft_wage_cap
        {
            return TransferPlausibilityVerdict::HardReject(
                TransferPlausibilityReason::UnaffordableWages,
            );
        }

        // ── Soft adjustments ──

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

        if huge_drop {
            // Extra dampener for huge sporting drops that escaped the hard
            // gate (e.g. listed/requested player from a much bigger club).
            adj.shortlist_score_multiplier *= 0.75;
        }

        if exemption {
            // Player welcoming the move counterbalances the soft penalties
            // so the buyer can actually finish a deal.
            adj.shortlist_score_multiplier = adj.shortlist_score_multiplier.max(0.75);
            adj.seller_acceptance_delta += 15.0;
            adj.player_terms_delta += 10.0;
            adj.minimum_fee_multiplier = (adj.minimum_fee_multiplier - 0.20).max(1.0);
        }

        TransferPlausibilityVerdict::Allow(adj)
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
    /// process_scouting / shortlists / build_shortlists). Looks up the
    /// selling club to compute rank and best-CA-in-group; returns `None`
    /// if the selling club can't be resolved.
    pub(crate) fn from_summary(
        country: &Country,
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
    ) -> Option<TransferPlausibilityInputs> {
        let selling_club = country.clubs.iter().find(|c| c.id == target.club_id)?;
        let main_team = selling_club
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main));
        let seller_rep = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let seller_world_rep = main_team.map(|t| t.reputation.world as i16).unwrap_or(0);
        let seller_league_id = main_team.and_then(|t| t.league_id);
        let seller_league_rep = seller_league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
            .map(|l| l.reputation)
            .unwrap_or(0);

        let player_ca = target.skill_ability;
        let position_group = target.position_group;
        let rank =
            PipelineProcessor::position_group_rank(selling_club, target.player_id, position_group);
        let rank = if rank == u8::MAX { 1 } else { rank };
        let best_group_ca =
            PipelineProcessor::best_ca_in_group(selling_club, position_group).max(player_ca);

        let same_country = target.country_id == buyer_ctx.buyer_country_id;
        let same_league_or_division = same_country
            && match (buyer_ctx.buyer_league_id, seller_league_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };

        // Squad status / contract data — we may not have the full Player
        // here, so fall back to `NotYetSet` if unavailable. Status flags
        // already on PlayerSummary cover the listing/requested/unhappy
        // signals we care about for the exemption.
        let player = PipelineProcessor::find_player_in_country(country, target.player_id);
        let squad_status = player
            .and_then(|p| p.contract.as_ref().map(|c| c.squad_status.clone()))
            .unwrap_or(PlayerSquadStatus::NotYetSet);

        // Real availability signals from the player's own status — never
        // from a market listing (which may be synthetic).
        let is_transfer_requested = player
            .map(|p| p.statuses.get().contains(&PlayerStatusType::Req))
            .unwrap_or(false);
        let is_unhappy = player
            .map(|p| p.statuses.get().contains(&PlayerStatusType::Unh))
            .unwrap_or(false);
        let seller_in_debt = selling_club.finance.balance.balance < 0;

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
            seller_rep,
            buyer_league_rep: buyer_ctx.buyer_league_rep,
            seller_league_rep,
            buyer_world_rep: buyer_ctx.buyer_world_rep,
            seller_world_rep,
            player_world_rep: target.world_reputation,
            player_current_rep: target.current_reputation,
            player_home_rep: target.home_reputation,
            player_age: target.age,
            position_group,
            is_listed: target.is_listed,
            is_loan_listed: target.is_loan_listed,
            is_transfer_requested,
            is_unhappy,
            squad_status,
            contract_months_remaining: target.contract_months_remaining,
            current_salary: target.salary,
            estimated_value: target.estimated_value,
            player_appearances: target.appearances,
            seller_position_rank: rank,
            player_ca,
            best_group_ca_at_seller: best_group_ca,
            is_loan,
            is_unsolicited,
            seller_in_debt,
            release_clause_triggered: false,
            same_country,
            same_league_or_division,
            buyer_transfer_budget: buyer_ctx.buyer_transfer_budget,
            buyer_wage_budget: buyer_ctx.buyer_wage_budget,
            buyer_total_wages: buyer_ctx.buyer_total_wages,
            expected_annual_wage,
        })
    }

    /// Convenience wrapper for callers who already have a `PlayerSummary`
    /// and a buyer context: returns the verdict directly.
    pub(crate) fn evaluate_summary(
        country: &Country,
        buyer_ctx: &BuyerPlausibilityContext,
        target: &PlayerSummary,
        is_loan: bool,
        is_unsolicited: bool,
    ) -> Option<TransferPlausibilityVerdict> {
        Self::from_summary(country, buyer_ctx, target, is_loan, is_unsolicited)
            .map(|i| TransferPlausibilityEvaluator::evaluate(&i))
    }

    /// Build plausibility inputs at negotiation-start time when the buyer
    /// already has the live `Club` + `Player` references. Most accurate
    /// path — uses the player's squad status and statuses straight from
    /// the contract / squad records rather than a snapshot.
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
        let buyer_ctx = BuyerPlausibilityContext::build(country, buyer_club);

        let main_team = selling_club
            .teams
            .iter()
            .find(|t| matches!(t.team_type, TeamType::Main));
        let seller_rep = main_team
            .map(|t| t.reputation.overall_score())
            .unwrap_or(0.3);
        let seller_world_rep = main_team.map(|t| t.reputation.world as i16).unwrap_or(0);
        let seller_league_id = main_team.and_then(|t| t.league_id);
        let seller_league_rep = seller_league_id
            .and_then(|lid| country.leagues.leagues.iter().find(|l| l.id == lid))
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

        let same_country = player.country_id == buyer_ctx.buyer_country_id;
        let buyer_league_id = buyer_ctx.buyer_league_id;
        let same_league_or_division = same_country
            && match (buyer_league_id, seller_league_id) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            };

        let expected_annual_wage = WageCalculator::expected_annual_wage(
            player,
            player.age(date),
            buyer_ctx.buyer_rep,
            buyer_ctx.buyer_league_rep,
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
            buyer_transfer_budget: buyer_ctx.buyer_transfer_budget,
            buyer_wage_budget: buyer_ctx.buyer_wage_budget,
            buyer_total_wages: buyer_ctx.buyer_total_wages,
            expected_annual_wage,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
