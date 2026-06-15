//! Market-exposure scoring for *available* players — the signed-side
//! analogue of the free-agent career-pressure model.
//!
//! The transfer pipeline already surfaces listed / requested / unhappy
//! players to plausible buyers (the "listed-star sweep" in
//! [`super::recommendations`]). What it lacked was a way to reason about
//! *how strongly* the market should be discovering a given available
//! player, *how stale* his availability has become, and *why* the market
//! has not bitten. This module supplies that, as pure, deterministic
//! scoring over observable signals so it is trivially unit-testable and
//! never reaches into the simulator world.
//!
//! Three outputs feed the pipeline:
//!   * `circulation_boost` lifts a stale, untouched player up the
//!     listed-sweep ranking so he doesn't vanish behind newer candidates.
//!   * `price_softening` / `wage_softening` model the seller dropping the
//!     asking price and the player relaxing his wage demand as the dry
//!     weeks accumulate — natural market feedback, not a forced transfer.
//!   * [`MarketDiscoveryDiagnosis`] reduces the per-buyer rejection
//!     reasons into a single [`AvailabilityBlockReason`] so a quality
//!     player with no interest accumulates a coherent explanation.
//!
//! Everything here is a method on a struct (no free functions) and every
//! type is reached through a `use` at the file header (no inline paths),
//! per project convention.

use crate::club::player::transfer::AvailabilityBlockReason;
use crate::transfers::pipeline::plausibility::TransferPlausibilityReason;
use crate::transfers::pipeline::recommendations::ListedRejectReason;

/// Qualitative band of an available player's position on the staleness
/// curve. Maps `days_available` onto the same kind of coarse stages the
/// free-agent [`crate::club::player::transfer::MarketStage`] uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::transfers::pipeline) enum ExposureStage {
    /// Just became available — give the market a fair look before reacting.
    Fresh,
    /// Establishing — some weeks in, the market has had a first pass.
    Building,
    /// Stale — months on the market with little to show; circulate wider.
    Stale,
    /// Stranded — a very long sit; maximum softening and circulation.
    Stranded,
}

impl ExposureStage {
    pub(in crate::transfers::pipeline) fn from_days(days: i64) -> Self {
        match days {
            d if d < 21 => ExposureStage::Fresh,
            d if d < 60 => ExposureStage::Building,
            d if d < 150 => ExposureStage::Stale,
            _ => ExposureStage::Stranded,
        }
    }
}

/// Observable signals describing a signed, available player and the
/// market context around him. Built by the pipeline from snapshots; kept
/// free of any `Player` borrow so [`AvailabilityExposure::compute`] stays
/// pure.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct AvailabilitySignals {
    /// Days since the player first became available in the current sit.
    pub days_available: i64,
    pub is_listed: bool,
    pub is_transfer_requested: bool,
    pub is_unhappy: bool,
    pub is_loan_listed: bool,
    pub current_ability: u8,
    pub estimated_potential: u8,
    pub age: u8,
    #[allow(dead_code)]
    pub estimated_value: f64,
    /// Asking price ÷ estimated value. > 1.0 = the seller is holding out
    /// for a premium; < 1.0 = already discounted.
    pub asking_to_value_ratio: f32,
    #[allow(dead_code)]
    pub current_salary: u32,
    #[allow(dead_code)]
    pub world_reputation: i16,
    /// Player ambition, 0..1 — pushes toward wanting a move.
    pub ambition: f32,
    /// Months left on contract; <= 6 reads as a bargain pickup.
    pub contract_months_remaining: i16,
    /// Seller is carrying a negative balance — distress-sale pressure.
    pub seller_in_debt: bool,
    /// Seller has a positional surplus at the player's group.
    pub squad_surplus: bool,
    /// A genuinely capable player who is barely playing — the market
    /// should want him even if his own club is ambivalent.
    pub low_usage_despite_ability: bool,
    /// Concrete approaches in the last 30 days (monitoring / shortlist /
    /// negotiation across all clubs).
    pub recent_interest_count: u8,
    /// Consecutive circulation scans that found no taker.
    pub failed_scans: u16,
}

/// The exposure verdict — a discoverability score plus the staleness
/// stage and the three behavioural feedback channels the pipeline reads.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct AvailabilityExposure {
    /// 0..100 — how strongly the market should be discovering this player.
    /// Diagnostic / ranking signal; not a gate.
    pub score: f32,
    pub stage: ExposureStage,
    /// Additive bonus folded into the listed-sweep soft score so stale,
    /// untouched players don't fall behind a churn of newer candidates.
    pub circulation_boost: f32,
    /// Fraction (0..0.30) the seller will quietly knock off the asking
    /// price after a long market failure.
    pub price_softening: f32,
    /// Fraction (0..0.25) the player relaxes his wage / level expectation
    /// over a dry spell.
    pub wage_softening: f32,
}

impl AvailabilityExposure {
    /// Score the market exposure of an available player. Pure and
    /// deterministic — same signals, same verdict.
    pub(in crate::transfers::pipeline) fn compute(s: &AvailabilitySignals) -> AvailabilityExposure {
        let stage = ExposureStage::from_days(s.days_available);

        // ── Discoverability score (0..~100) ──
        let mut score = 0.0_f32;
        // Quality draws market attention.
        score += (s.current_ability as f32 / 200.0) * 35.0;
        // Youth upside.
        if s.age <= 23 && s.estimated_potential > s.current_ability {
            score += ((s.estimated_potential - s.current_ability) as f32).clamp(0.0, 20.0) * 0.5;
        }
        // Status urgency — a player pushing to leave is more on the market
        // than one merely loan-listed.
        score += if s.is_transfer_requested {
            12.0
        } else if s.is_listed {
            9.0
        } else if s.is_unhappy {
            7.0
        } else if s.is_loan_listed {
            5.0
        } else {
            0.0
        };
        // Staleness raises the urgency to find SOME outcome (capped ~1yr).
        let staleness = (s.days_available as f32 / 30.0).clamp(0.0, 12.0);
        score += staleness * 1.6;
        score += (s.failed_scans as f32).clamp(0.0, 12.0) * 0.8;
        // A capable player who isn't playing should move.
        if s.low_usage_despite_ability {
            score += 8.0;
        }
        // Seller-side pressure.
        if s.seller_in_debt {
            score += 4.0;
        }
        if s.squad_surplus {
            score += 4.0;
        }
        // Ambition pushes toward a move.
        score += s.ambition.clamp(0.0, 1.0) * 5.0;
        // Bargain signals lift discoverability.
        if s.contract_months_remaining > 0 && s.contract_months_remaining <= 6 {
            score += 5.0;
        }
        if s.asking_to_value_ratio < 0.95 {
            score += 3.0;
        }
        // Already being pursued — less need for the market to "discover" him.
        score -= (s.recent_interest_count as f32).clamp(0.0, 6.0) * 2.5;
        let score = score.clamp(0.0, 100.0);

        // ── Circulation boost for the listed-sweep soft score ──
        let stage_boost = match stage {
            ExposureStage::Fresh => 0.0,
            ExposureStage::Building => 2.0,
            ExposureStage::Stale => 5.0,
            ExposureStage::Stranded => 8.0,
        };
        // An untouched player gains extra lift the longer the silence runs.
        let no_interest_boost = if s.recent_interest_count == 0 {
            (s.failed_scans as f32).clamp(0.0, 6.0) * 0.6
        } else {
            0.0
        };
        let circulation_boost = (stage_boost + no_interest_boost).clamp(0.0, 12.0);

        // ── Softening curves ──
        let stale_frac = (s.days_available as f32 / 365.0).clamp(0.0, 1.0);
        let scan_frac = (s.failed_scans as f32 / 12.0).clamp(0.0, 1.0);
        // Seller drops the price the longer it sits — more so when the tag
        // sits above the player's value.
        let overpriced = (s.asking_to_value_ratio - 1.0).clamp(0.0, 0.6);
        let price_softening =
            (stale_frac * 0.18 + scan_frac * 0.08 + overpriced * 0.10).clamp(0.0, 0.30);
        // The player relaxes his wage / level demand over a dry spell; a
        // player who actually handed in a request softens a touch faster.
        let request_bonus = if s.is_transfer_requested { 0.03 } else { 0.0 };
        let wage_softening =
            (stale_frac * 0.14 + scan_frac * 0.07 + request_bonus).clamp(0.0, 0.25);

        AvailabilityExposure {
            score,
            stage,
            circulation_boost,
            price_softening,
            wage_softening,
        }
    }
}

/// Reduces per-buyer listed-target rejections into a single, durable
/// [`AvailabilityBlockReason`]. Stateless namespace (no free functions).
pub(in crate::transfers::pipeline) struct MarketDiscoveryDiagnosis;

impl MarketDiscoveryDiagnosis {
    /// Map one per-buyer listed-target rejection into the player-centric
    /// block-reason taxonomy.
    pub(in crate::transfers::pipeline) fn from_listed_reject(
        reason: ListedRejectReason,
    ) -> AvailabilityBlockReason {
        match reason {
            // The player carries no availability flag from this buyer's
            // point of view — too early / not really on the market.
            ListedRejectReason::NotListed => AvailabilityBlockReason::TooEarly,
            ListedRejectReason::OutOfTierWindow => AvailabilityBlockReason::ReputationTooHigh,
            ListedRejectReason::UnaffordableFee => AvailabilityBlockReason::AskingPriceTooHigh,
            ListedRejectReason::UnaffordableWage => AvailabilityBlockReason::WageTooHigh,
            ListedRejectReason::ReputationGapTooLarge => AvailabilityBlockReason::ReputationTooHigh,
            ListedRejectReason::NoSquadNeed => AvailabilityBlockReason::NoAffordableSquadNeed,
            ListedRejectReason::NotAnUpgrade => AvailabilityBlockReason::NoAffordableSquadNeed,
        }
    }

    /// Map a plausibility hard-reject (the realism wall a club hit on an
    /// otherwise affordable, in-tier target) into the block-reason
    /// taxonomy. Covers the country/region and step-down cases the
    /// affordability-only `from_listed_reject` can't express.
    pub(in crate::transfers::pipeline) fn from_plausibility(
        reason: TransferPlausibilityReason,
    ) -> AvailabilityBlockReason {
        match reason {
            TransferPlausibilityReason::CountryPairBlocked => {
                AvailabilityBlockReason::CountryRegionBlocked
            }
            TransferPlausibilityReason::DomesticStepDownForPrimeStarter
            | TransferPlausibilityReason::ImportantPlayerAtMuchStrongerClub
            | TransferPlausibilityReason::LoanNotCredible => {
                AvailabilityBlockReason::PlayerWontStepDown
            }
            TransferPlausibilityReason::UnaffordableWages => AvailabilityBlockReason::WageTooHigh,
            TransferPlausibilityReason::UnaffordableFee => {
                AvailabilityBlockReason::AskingPriceTooHigh
            }
            TransferPlausibilityReason::NoSportingUpside => {
                AvailabilityBlockReason::NoAffordableSquadNeed
            }
        }
    }

    /// Reduce a tally of per-buyer block reasons into the single dominant
    /// explanation. Picks the most frequent wall the market hit, breaking
    /// ties toward the closer-to-a-deal funnel stage (higher `rank`).
    /// `NoPlausibleBuyer` when no buyer was even in tier.
    pub(in crate::transfers::pipeline) fn dominant(
        rejects: &[AvailabilityBlockReason],
    ) -> AvailabilityBlockReason {
        if rejects.is_empty() {
            return AvailabilityBlockReason::NoPlausibleBuyer;
        }
        let mut seen: Vec<AvailabilityBlockReason> = Vec::new();
        for r in rejects {
            if !seen.contains(r) {
                seen.push(*r);
            }
        }
        let mut best: Option<(AvailabilityBlockReason, usize)> = None;
        for cand in seen {
            let count = rejects.iter().filter(|r| **r == cand).count();
            let better = match best {
                None => true,
                Some((b, bc)) => count > bc || (count == bc && cand.rank() > b.rank()),
            };
            if better {
                best = Some((cand, count));
            }
        }
        best.map(|(r, _)| r)
            .unwrap_or(AvailabilityBlockReason::NoPlausibleBuyer)
    }
}

/// Observable signals for a (soon-)free player the opportunistic scout
/// evaluates. Covers both the genuine pool free agent (career pressure
/// above zero) and the domestic player whose contract is running down
/// (career pressure zero, `contract_months_remaining` small).
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct FreeAgentRecommendationSignals {
    pub current_ability: u8,
    pub estimated_potential: u8,
    pub age: u8,
    /// Months left on contract; 0 = already a free agent.
    pub contract_months_remaining: u32,
    /// Career-pressure score 0..1 — 0 for a still-contracted player.
    pub career_pressure: f32,
}

/// Buyer-side context for the opportunistic free-agent decision.
#[derive(Debug, Clone, Copy)]
pub(in crate::transfers::pipeline) struct FreeAgentBuyerContext {
    pub buyer_avg_ability: u8,
    /// Club has roster room (below its registration / squad-size cap).
    pub buyer_squad_room: bool,
    /// Annual wage budget minus committed wages — a token positive value
    /// is enough to fund a depth/free signing.
    pub buyer_wage_headroom: i64,
    /// The player's position group is thin at this club (below depth).
    pub group_below_depth: bool,
}

/// Pure decision: should a plausible club file an opportunistic
/// free-agent / soon-free recommendation for this player? The wiring
/// pass supplies the buyer context and owns caps / dedup / plausibility.
/// This keeps the "natural discovery" judgement in one tested place
/// rather than as inline conditions scattered through the recommender.
pub(in crate::transfers::pipeline) struct OpportunisticFreeAgentScout;

impl OpportunisticFreeAgentScout {
    pub(in crate::transfers::pipeline) fn should_recommend(
        fa: &FreeAgentRecommendationSignals,
        buyer: &FreeAgentBuyerContext,
    ) -> bool {
        // Need somewhere to register him and something to pay him.
        if !buyer.buyer_squad_room || buyer.buyer_wage_headroom <= 0 {
            return false;
        }
        // Usefulness: depth-relevant for this club, a genuine prospect, or
        // plugging a thin group. A clearly sub-standard player is left to
        // sit (low-quality free agents may remain unsigned — by design).
        let depth_relevant = fa.current_ability + 12 >= buyer.buyer_avg_ability;
        let prospect = fa.age <= 21 && fa.estimated_potential >= buyer.buyer_avg_ability;
        let fills_gap =
            buyer.group_below_depth && fa.current_ability + 18 >= buyer.buyer_avg_ability;
        if !(depth_relevant || prospect || fills_gap) {
            return false;
        }
        // Reputation discipline without hardcoded tiers: a player well
        // above the club's level won't drop in on a free unless he's been
        // unemployed long enough to be flexible (rising career pressure).
        let well_above = fa.current_ability > buyer.buyer_avg_ability + 25;
        if well_above && fa.career_pressure < 0.5 {
            return false;
        }
        // Soon-free or already-free only — a player years from the end of
        // his deal isn't a free-agent target.
        let nearly_free = fa.contract_months_remaining == 0 || fa.contract_months_remaining <= 6;
        if !nearly_free {
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Baseline signals for a mid-quality, just-listed player. Tests tweak
    /// individual axes off this. Wrapped in a unit struct (no free fns).
    struct ExposureFixtures;

    impl ExposureFixtures {
        fn fresh_listed() -> AvailabilitySignals {
            AvailabilitySignals {
                days_available: 5,
                is_listed: true,
                is_transfer_requested: false,
                is_unhappy: false,
                is_loan_listed: false,
                current_ability: 120,
                estimated_potential: 125,
                age: 27,
                estimated_value: 5_000_000.0,
                asking_to_value_ratio: 1.0,
                current_salary: 400_000,
                world_reputation: 4000,
                ambition: 0.5,
                contract_months_remaining: 30,
                seller_in_debt: false,
                squad_surplus: false,
                low_usage_despite_ability: false,
                recent_interest_count: 0,
                failed_scans: 0,
            }
        }
    }

    #[test]
    fn stage_thresholds() {
        assert_eq!(ExposureStage::from_days(0), ExposureStage::Fresh);
        assert_eq!(ExposureStage::from_days(20), ExposureStage::Fresh);
        assert_eq!(ExposureStage::from_days(21), ExposureStage::Building);
        assert_eq!(ExposureStage::from_days(59), ExposureStage::Building);
        assert_eq!(ExposureStage::from_days(60), ExposureStage::Stale);
        assert_eq!(ExposureStage::from_days(149), ExposureStage::Stale);
        assert_eq!(ExposureStage::from_days(150), ExposureStage::Stranded);
    }

    #[test]
    fn circulation_boost_grows_with_staleness() {
        // A stale, requested player gets wider circulation over time —
        // the spec's "stale requested player gets wider circulation".
        let mut fresh = ExposureFixtures::fresh_listed();
        fresh.is_transfer_requested = true;
        fresh.is_listed = false;

        let mut stale = fresh;
        stale.days_available = 100;
        stale.failed_scans = 6;

        let mut stranded = fresh;
        stranded.days_available = 300;
        stranded.failed_scans = 10;

        let fb = AvailabilityExposure::compute(&fresh).circulation_boost;
        let sb = AvailabilityExposure::compute(&stale).circulation_boost;
        let xb = AvailabilityExposure::compute(&stranded).circulation_boost;
        assert!(fb < sb, "stale must out-circulate fresh: {fb} !< {sb}");
        assert!(sb < xb, "stranded must out-circulate stale: {sb} !< {xb}");
    }

    #[test]
    fn softening_grows_with_failed_market_weeks() {
        // Asking price AND wage expectation soften naturally after long
        // market failure — the spec's softening requirement.
        let fresh = ExposureFixtures::fresh_listed();
        let mut long_failed = fresh;
        long_failed.is_transfer_requested = true;
        long_failed.days_available = 300;
        long_failed.failed_scans = 10;
        long_failed.asking_to_value_ratio = 1.3;

        let e0 = AvailabilityExposure::compute(&fresh);
        let e1 = AvailabilityExposure::compute(&long_failed);
        assert!(
            e1.price_softening > e0.price_softening,
            "price must soften after long failure: {} !> {}",
            e1.price_softening,
            e0.price_softening
        );
        assert!(
            e1.wage_softening > e0.wage_softening,
            "wage must soften after long failure: {} !> {}",
            e1.wage_softening,
            e0.wage_softening
        );
        // Softening stays bounded — never a giveaway.
        assert!(e1.price_softening <= 0.30 && e1.wage_softening <= 0.25);
    }

    #[test]
    fn stale_availability_softens_gradually_not_instantly() {
        // Spec: stale availability softens price/wage GRADUALLY, never
        // instantly — a first-team player isn't suddenly a giveaway the
        // week he's listed.
        let mut s = ExposureFixtures::fresh_listed();
        s.is_transfer_requested = true;
        s.is_listed = false;

        // Just available → essentially no softening yet.
        let fresh = AvailabilityExposure::compute(&s);
        assert!(
            fresh.price_softening < 0.05 && fresh.wage_softening < 0.06,
            "fresh availability must barely soften: price {} wage {}",
            fresh.price_softening,
            fresh.wage_softening
        );

        // A month in, one dry scan → a little more.
        let mut month = s;
        month.days_available = 30;
        month.failed_scans = 1;
        let mid = AvailabilityExposure::compute(&month);
        assert!(mid.price_softening > fresh.price_softening);

        // A full year stranded with a dozen dry scans → near the cap but
        // still strictly bounded (never a giveaway).
        let mut year = s;
        year.days_available = 365;
        year.failed_scans = 12;
        let long = AvailabilityExposure::compute(&year);
        assert!(long.price_softening > mid.price_softening);
        assert!(long.price_softening <= 0.30 && long.wage_softening <= 0.25);
    }

    #[test]
    fn recent_interest_suppresses_exposure() {
        // A player several clubs are already chasing needs less "market
        // discovery" help than an identical, untouched one.
        let untouched = ExposureFixtures::fresh_listed();
        let mut pursued = untouched;
        pursued.recent_interest_count = 4;
        assert!(
            AvailabilityExposure::compute(&pursued).score
                < AvailabilityExposure::compute(&untouched).score
        );
    }

    #[test]
    fn quality_and_idle_quality_raise_score() {
        let base = ExposureFixtures::fresh_listed();
        let mut idle_star = base;
        idle_star.current_ability = 150;
        idle_star.low_usage_despite_ability = true;
        assert!(
            AvailabilityExposure::compute(&idle_star).score
                > AvailabilityExposure::compute(&base).score
        );
    }

    #[test]
    fn diagnosis_maps_reject_reasons() {
        assert_eq!(
            MarketDiscoveryDiagnosis::from_listed_reject(ListedRejectReason::UnaffordableFee),
            AvailabilityBlockReason::AskingPriceTooHigh
        );
        assert_eq!(
            MarketDiscoveryDiagnosis::from_listed_reject(ListedRejectReason::UnaffordableWage),
            AvailabilityBlockReason::WageTooHigh
        );
        assert_eq!(
            MarketDiscoveryDiagnosis::from_listed_reject(ListedRejectReason::ReputationGapTooLarge),
            AvailabilityBlockReason::ReputationTooHigh
        );
        assert_eq!(
            MarketDiscoveryDiagnosis::from_listed_reject(ListedRejectReason::NoSquadNeed),
            AvailabilityBlockReason::NoAffordableSquadNeed
        );
    }

    #[test]
    fn dominant_picks_most_frequent_then_higher_rank() {
        // Empty tally → no plausible buyer at all.
        assert_eq!(
            MarketDiscoveryDiagnosis::dominant(&[]),
            AvailabilityBlockReason::NoPlausibleBuyer
        );
        // Most frequent wins.
        let tally = [
            AvailabilityBlockReason::AskingPriceTooHigh,
            AvailabilityBlockReason::AskingPriceTooHigh,
            AvailabilityBlockReason::WageTooHigh,
        ];
        assert_eq!(
            MarketDiscoveryDiagnosis::dominant(&tally),
            AvailabilityBlockReason::AskingPriceTooHigh
        );
        // Tie broken toward the closer-to-a-deal funnel stage (higher rank):
        // WageTooHigh (rank 6) beats ReputationTooHigh (rank 3).
        let tied = [
            AvailabilityBlockReason::ReputationTooHigh,
            AvailabilityBlockReason::WageTooHigh,
        ];
        assert_eq!(
            MarketDiscoveryDiagnosis::dominant(&tied),
            AvailabilityBlockReason::WageTooHigh
        );
    }

    #[test]
    fn opportunistic_scout_recommends_useful_affordable_free_agent() {
        // A useful, affordable soon-free player at a club with room — the
        // spec's "useful free agent without a request becomes a rec".
        let fa = FreeAgentRecommendationSignals {
            current_ability: 95,
            estimated_potential: 100,
            age: 27,
            contract_months_remaining: 4,
            career_pressure: 0.0,
        };
        let buyer = FreeAgentBuyerContext {
            buyer_avg_ability: 96,
            buyer_squad_room: true,
            buyer_wage_headroom: 500_000,
            group_below_depth: true,
        };
        assert!(OpportunisticFreeAgentScout::should_recommend(&fa, &buyer));
    }

    #[test]
    fn opportunistic_scout_skips_low_quality_free_agent() {
        // A clearly sub-standard free agent is left unsigned (low-quality
        // free agents may remain unsigned — by design).
        let fa = FreeAgentRecommendationSignals {
            current_ability: 55,
            estimated_potential: 58,
            age: 30,
            contract_months_remaining: 0,
            career_pressure: 0.4,
        };
        let buyer = FreeAgentBuyerContext {
            buyer_avg_ability: 100,
            buyer_squad_room: true,
            buyer_wage_headroom: 1_000_000,
            group_below_depth: false,
        };
        assert!(!OpportunisticFreeAgentScout::should_recommend(&fa, &buyer));
    }

    #[test]
    fn opportunistic_scout_skips_when_no_room_or_no_wage() {
        let fa = FreeAgentRecommendationSignals {
            current_ability: 100,
            estimated_potential: 105,
            age: 26,
            contract_months_remaining: 0,
            career_pressure: 0.5,
        };
        let no_room = FreeAgentBuyerContext {
            buyer_avg_ability: 100,
            buyer_squad_room: false,
            buyer_wage_headroom: 1_000_000,
            group_below_depth: true,
        };
        let no_wage = FreeAgentBuyerContext {
            buyer_avg_ability: 100,
            buyer_squad_room: true,
            buyer_wage_headroom: -1,
            group_below_depth: true,
        };
        assert!(!OpportunisticFreeAgentScout::should_recommend(
            &fa, &no_room
        ));
        assert!(!OpportunisticFreeAgentScout::should_recommend(
            &fa, &no_wage
        ));
    }

    #[test]
    fn opportunistic_scout_blocks_star_drop_without_pressure() {
        // A player well above the club's level won't drop in on a free
        // unless he's been unemployed long enough to be flexible.
        let star = FreeAgentRecommendationSignals {
            current_ability: 140,
            estimated_potential: 145,
            age: 29,
            contract_months_remaining: 0,
            career_pressure: 0.2,
        };
        let small_club = FreeAgentBuyerContext {
            buyer_avg_ability: 100,
            buyer_squad_room: true,
            buyer_wage_headroom: 5_000_000,
            group_below_depth: true,
        };
        assert!(!OpportunisticFreeAgentScout::should_recommend(
            &star,
            &small_club
        ));
        // Same player after a long unemployment (high pressure) becomes a
        // plausible, flexible target.
        let mut desperate = star;
        desperate.career_pressure = 0.7;
        assert!(OpportunisticFreeAgentScout::should_recommend(
            &desperate,
            &small_club
        ));
    }
}
