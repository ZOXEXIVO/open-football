//! Tunables for the scouting subsystem.
//!
//! Before this file existed, ~25 magic numbers — observation chances,
//! error multipliers, region penalties, recommendation thresholds,
//! risk-flag cutoffs, shadow caps — sat as inline literals scattered
//! across `scouting.rs`, `helpers.rs`, and parts of `pipeline/mod.rs`.
//! Tuning balance meant grepping; A/B-ing a difficulty curve was
//! impossible.
//!
//! All knobs now live here, grouped by concern, with helper methods
//! covering the formulas the rest of the pipeline used to inline. The
//! config is passed by reference into the few entry points that need
//! it (`process_scouting`, `process_match_scouting`, etc.) so that
//! per-save overrides can be plumbed in later without re-touching the
//! call sites.

use crate::PlayerSquadStatus;
use crate::transfers::ReportRiskFlag;
use crate::transfers::pipeline::plausibility::EffectivePlayerReputation;
use crate::transfers::pipeline::{PlayerSummary, ScoutingRecommendation};

// ============================================================
// Sub-configs grouped by concern
// ============================================================

#[derive(Debug, Clone)]
pub struct ObservationConfig {
    /// Daily probability that a scout makes any observation: percentage =
    /// `base_pct + judging_ability * skill_factor`. Caps at 100 implicitly
    /// (the roll is `random(0,100) > chance`).
    pub daily_chance_base_pct: i32,
    pub daily_chance_skill_factor: f32,

    /// Per-day observation budget: `floor + judging_ability / divisor`.
    pub per_day_floor: usize,
    pub per_day_skill_divisor: usize,

    /// Probability of re-observing a previously seen target rather than
    /// discovering a new one (deepens existing reports vs widens the pool).
    pub re_observe_chance_pct: i32,

    /// Default judging values used when an assignment has no scout
    /// (manager-led) and when a scout staff record cannot be located.
    /// These are deliberately different — the absent-scout case is "no
    /// scouting department, manager pitches in" while the missing-staff
    /// case is "we lost the pointer, default conservatively."
    pub default_judging_when_no_scout: u8,
    pub default_judging_when_staff_missing: u8,
}

#[derive(Debug, Clone)]
pub struct ErrorConfig {
    /// `max_judging - judging_skill` is the per-observation random error
    /// width (clamped at `min_error`). Match-context observations apply
    /// `match_multiplier`; otherwise full error.
    pub max_judging: i16,
    pub min_error: i16,
    pub match_multiplier: f32,
}

#[derive(Debug, Clone)]
pub struct RegionConfig {
    /// Domestic observations carry no penalty.
    pub domestic_penalty: f32,
    /// Foreign observation in a region the scout is "known to."
    pub known_foreign_penalty: f32,
    /// Foreign observation outside any known region.
    pub unknown_foreign_penalty: f32,
    /// Familiarity (0–100) reduces the effective penalty:
    /// `penalty = (base - familiarity / divisor).max(floor)`.
    pub familiarity_divisor: f32,
    pub penalty_floor: f32,
}

#[derive(Debug, Clone)]
pub struct RecommendationConfig {
    /// Effective ability ≥ `min_ability + strong_buy_gap` AND
    /// potential > ability + `strong_buy_potential_gap` ⇒ StrongBuy.
    pub strong_buy_gap: i16,
    pub strong_buy_potential_gap: u8,
    /// Effective ability ≥ `min_ability` AND potential ≥ ability ⇒ Buy.
    pub buy_gap: i16,
    /// Effective ability ≥ `min_ability - consider_gap` ⇒ Consider; else Pass.
    pub consider_gap: i16,

    /// Youth-bonus tiers added to effective ability before recommendation.
    /// Both rules require the player to be young AND have a meaningful
    /// gap between assessed potential and assessed current ability.
    pub youth_tier1_age_max: u8,
    pub youth_tier1_potential_gap_min: u8,
    pub youth_tier1_bonus: i16,
    pub youth_tier2_age_max: u8,
    pub youth_tier2_potential_gap_min: u8,
    pub youth_tier2_bonus: i16,

    /// Stats-bonus tiers added when the player has played and rated well.
    pub stats_tier1_apps_min: u16,
    pub stats_tier1_rating_min: f32,
    pub stats_tier1_bonus: i16,
    pub stats_tier2_apps_min: u16,
    pub stats_tier2_rating_min: f32,
    pub stats_tier2_bonus: i16,

    /// Performance-bonus tiers applied to the *assessed* ability during
    /// observation (separate from recommendation-time stats bonus). High
    /// apps + good rating implies the visible ability is reliable.
    pub perf_high_apps_min: u16,
    pub perf_high_rating_min: f32,
    pub perf_high_bonus: i32,
    pub perf_med_apps_min: u16,
    pub perf_med_rating_min: f32,
    pub perf_med_bonus: i32,
    pub perf_poor_rating_max: f32,
    pub perf_poor_bonus: i32,

    /// Match-rating performance bonus (applied during match-day observations).
    pub match_rating_excellent: f32,
    pub match_rating_excellent_bonus: i32,
    pub match_rating_good: f32,
    pub match_rating_good_bonus: i32,
    pub match_rating_ok: f32,
    pub match_rating_ok_bonus: i32,
    pub match_rating_poor_max: f32,
    pub match_rating_poor_bonus: i32,
}

#[derive(Debug, Clone)]
pub struct RiskFlagConfig {
    /// `determination < threshold` → PoorAttitude.
    pub poor_attitude_determination_max: f32,
    /// `age >= threshold` → AgeRisk.
    pub age_risk_min: u8,
    /// `0 < contract_months <= threshold` → ContractExpiring.
    pub contract_expiring_months_max: i16,
    /// `player_rep > buyer_rep + threshold` → WageDemands.
    pub wage_demands_rep_gap: i16,
}

#[derive(Debug, Clone)]
pub struct ShadowConfig {
    /// Hard cap on shadow reports kept per (position group, club).
    pub cap_per_group: usize,
    /// Hard cap on the known-player memory list per club.
    pub known_player_cap: usize,

    /// Confidence multiplier when seeding a fresh assignment from a
    /// shadow report at window-open. Reflects how much we trust an
    /// observation made during the previous window.
    pub seed_confidence_decay: f32,
    /// Floor / ceiling on the seeded confidence after decay.
    pub seed_confidence_floor: f32,
    pub seed_confidence_ceiling: f32,

    /// Weekly refresh rate: refresh `(len/divisor).max(min).min(max)`
    /// shadow reports per club per Monday.
    pub refresh_count_divisor: usize,
    pub refresh_count_min: usize,
    pub refresh_count_max: usize,

    /// Default judging used when no scouts are available during refresh.
    pub refresh_default_judging: u8,
}

#[derive(Debug, Clone)]
pub struct DataPrefilterConfig {
    /// Below this candidate count the data department doesn't bother.
    pub min_pool_size: usize,
    /// Pool size when the club's data analyst is at zero skill — wide net.
    pub pool_size_at_zero_skill: usize,
    /// Pool size shrinkage per data-skill point. Effective floor at 25.
    pub pool_size_per_skill_step: usize,
    /// Lower bound on the narrowed pool.
    pub pool_size_floor: usize,
    /// Random jitter when ranking candidates by data score:
    /// `random(-noise, noise)` where `noise = max_data - skill`.
    pub max_data_skill: i32,
    /// Default data-analysis skill when no scouts exist.
    pub default_data_skill: u8,
}

#[derive(Debug, Clone)]
pub struct RealismConfig {
    /// World-rep gap above which the target's club is "much bigger" than
    /// the buyer's. Reputation is on a 0-10000 scale, so ~2000 ≈ one full
    /// reputation tier (Regional → National, National → Continental).
    pub club_rep_gap_blocking: i16,
    /// Widened club-rep gap that bounds the listed / loan-listed exemption.
    /// A player his club is willing to sell or loan out is reachable by
    /// smaller clubs — but availability does not erase the level gap. A club
    /// more than this far below the seller (≈ two tiers) still cannot
    /// realistically host, pay, or attract a top-club player even on a loan,
    /// so it falls through to the normal first-team-regular / prominence
    /// blocks instead of being waved straight through. Wider than
    /// `club_rep_gap_blocking` because being listed genuinely does widen a
    /// player's reach — just not without limit.
    pub listed_exemption_gap_blocking: i16,
    /// Player world-rep gap above which the player himself is too prominent
    /// for the buyer's level. A player past this gap is hard-blocked as a
    /// first-team regular at a much bigger club — neither youth nor an
    /// expiring contract rescues that case.
    pub player_rep_gap_blocking: i16,
    /// Season-appearance count at/above which a player counts as a
    /// first-team regular. Below this they read as a backup/fringe player
    /// and stay attainable even from a much bigger club.
    pub first_team_regular_apps: u16,
    /// Contract months remaining at/below which the near-free exemption
    /// can fire — additionally gated by the affordability check.
    pub near_free_contract_months: i16,
    /// Age at/below which a young prospect can pass the realism gate as a
    /// development signing — only fires when the player is *not* already
    /// individually prominent. A 21-y-o star at a giant club is still
    /// blocked because the prominence check runs first.
    pub youth_exempt_age_max: u8,
    /// Linear cap on annual salary the buyer can absorb, scaled by world
    /// reputation. `max_salary = buyer_world_rep * salary_per_rep_point`.
    /// Defaults are calibrated so a tier-3 buyer (rep ~3500) can afford
    /// mid-five-figure-monthly wages but not top-club star wages.
    pub salary_per_rep_point: f64,
    /// Linear cap on estimated_value the buyer can absorb, scaled by
    /// world reputation. Used together with the salary cap to gate the
    /// near-free contract exemption.
    pub value_per_rep_point: f64,
}

#[derive(Debug, Clone)]
pub struct AssignmentConfig {
    /// Max scouts assigned to youth/reserve match observation per club.
    pub max_match_assignments_per_club: usize,
    /// Months a "Pass" recommendation blocks re-scouting the same player.
    pub rejection_memory_months: i64,
    /// Match-context: how many observations before a report is produced.
    pub match_report_threshold: u8,
    /// Pool-context: how many observations before a report is produced.
    /// Currently advisory — the pool path generates after every observation
    /// (low-confidence reports filter themselves out via the recommendation
    /// tier). Kept here so a future tightening lands in one place.
    #[allow(dead_code)]
    pub pool_report_threshold: u8,
    /// Confidence assigned when a pool report is generated from the
    /// very first observation (below the normal `1 - 1/(n+1)` curve).
    pub single_observation_confidence: f32,
}

// ============================================================
// Top-level config
// ============================================================

#[derive(Debug, Clone)]
pub struct ScoutingConfig {
    pub observation: ObservationConfig,
    pub error: ErrorConfig,
    pub region: RegionConfig,
    pub recommendation: RecommendationConfig,
    pub risk_flags: RiskFlagConfig,
    pub shadow: ShadowConfig,
    pub data_prefilter: DataPrefilterConfig,
    pub realism: RealismConfig,
    pub assignment: AssignmentConfig,
}

impl Default for ScoutingConfig {
    fn default() -> Self {
        ScoutingConfig {
            observation: ObservationConfig {
                daily_chance_base_pct: 60,
                daily_chance_skill_factor: 0.5,
                per_day_floor: 2,
                per_day_skill_divisor: 10,
                re_observe_chance_pct: 60,
                default_judging_when_no_scout: 8,
                default_judging_when_staff_missing: 10,
            },
            error: ErrorConfig {
                max_judging: 20,
                min_error: 1,
                match_multiplier: 0.6,
            },
            region: RegionConfig {
                domestic_penalty: 1.0,
                known_foreign_penalty: 1.0,
                unknown_foreign_penalty: 1.5,
                familiarity_divisor: 200.0,
                penalty_floor: 0.5,
            },
            recommendation: RecommendationConfig {
                strong_buy_gap: 10,
                strong_buy_potential_gap: 5,
                buy_gap: 0,
                consider_gap: 5,

                youth_tier1_age_max: 21,
                youth_tier1_potential_gap_min: 15,
                youth_tier1_bonus: 10,
                youth_tier2_age_max: 23,
                youth_tier2_potential_gap_min: 10,
                youth_tier2_bonus: 5,

                stats_tier1_apps_min: 10,
                stats_tier1_rating_min: 7.0,
                stats_tier1_bonus: 5,
                stats_tier2_apps_min: 5,
                stats_tier2_rating_min: 6.5,
                stats_tier2_bonus: 2,

                perf_high_apps_min: 10,
                perf_high_rating_min: 7.0,
                perf_high_bonus: 3,
                perf_med_apps_min: 5,
                perf_med_rating_min: 6.5,
                perf_med_bonus: 1,
                perf_poor_rating_max: 5.5,
                perf_poor_bonus: -2,

                match_rating_excellent: 7.5,
                match_rating_excellent_bonus: 5,
                match_rating_good: 7.0,
                match_rating_good_bonus: 3,
                match_rating_ok: 6.5,
                match_rating_ok_bonus: 1,
                match_rating_poor_max: 5.5,
                match_rating_poor_bonus: -3,
            },
            risk_flags: RiskFlagConfig {
                poor_attitude_determination_max: 8.0,
                age_risk_min: 31,
                contract_expiring_months_max: 6,
                wage_demands_rep_gap: 1500,
            },
            shadow: ShadowConfig {
                cap_per_group: 15,
                known_player_cap: 120,
                seed_confidence_decay: 0.7,
                seed_confidence_floor: 0.2,
                seed_confidence_ceiling: 1.0,
                refresh_count_divisor: 5,
                refresh_count_min: 1,
                refresh_count_max: 3,
                refresh_default_judging: 10,
            },
            data_prefilter: DataPrefilterConfig {
                min_pool_size: 20,
                pool_size_at_zero_skill: 80,
                pool_size_per_skill_step: 3,
                pool_size_floor: 25,
                max_data_skill: 20,
                default_data_skill: 8,
            },
            realism: RealismConfig {
                club_rep_gap_blocking: 2000,
                listed_exemption_gap_blocking: 4500,
                player_rep_gap_blocking: 1500,
                first_team_regular_apps: 15,
                near_free_contract_months: 12,
                youth_exempt_age_max: 21,
                salary_per_rep_point: 500.0,
                value_per_rep_point: 3000.0,
            },
            assignment: AssignmentConfig {
                max_match_assignments_per_club: 3,
                rejection_memory_months: 6,
                match_report_threshold: 2,
                pool_report_threshold: 1,
                single_observation_confidence: 0.4,
            },
        }
    }
}

// ============================================================
// Realism-gate target shape
// ============================================================

/// The minimal target shape the realism gate reasons about. Lets the
/// `PlayerSummary`-based scouting pool and the raw-`Player` match-scouting
/// path share one realism implementation instead of drifting apart.
#[derive(Debug, Clone)]
pub struct RealismTarget {
    pub club_world_reputation: i16,
    pub world_reputation: i16,
    pub current_reputation: i16,
    pub home_reputation: i16,
    pub appearances: u16,
    pub age: u8,
    pub contract_months_remaining: i16,
    pub salary: u32,
    pub estimated_value: f64,
    pub is_listed: bool,
    pub is_loan_listed: bool,
    /// Declared squad role at the selling club. A backup / not-needed player
    /// is attainable from a much bigger club; a first-team regular or key
    /// player is not.
    pub squad_status: PlayerSquadStatus,
}

impl RealismTarget {
    /// True when the player's declared role makes him surplus to the selling
    /// club — a backup or an unwanted player a smaller club could realistically
    /// take. First-team regulars, key players, and rotation options are not
    /// surplus (a `NotYetSet` status is treated as not-surplus — absence of a
    /// role is not evidence of one).
    fn is_squad_surplus(&self) -> bool {
        matches!(
            self.squad_status,
            PlayerSquadStatus::MainBackupPlayer | PlayerSquadStatus::NotNeeded
        )
    }
}

// ============================================================
// Pure helpers — formulas the pipeline used to inline
// ============================================================

impl ScoutingConfig {
    /// Daily percentage chance that a scout records any observation.
    pub fn daily_observation_chance(&self, judging_ability: u8) -> i32 {
        self.observation.daily_chance_base_pct
            + (judging_ability as f32 * self.observation.daily_chance_skill_factor) as i32
    }

    /// How many observations a scout attempts on a given day.
    pub fn observations_per_day(&self, judging_ability: u8) -> usize {
        self.observation.per_day_floor
            + judging_ability as usize / self.observation.per_day_skill_divisor
    }

    /// Per-observation random error width (one-sided `random(-w, w)`).
    /// `is_match` flag selects the match-context multiplier.
    pub fn effective_error(
        &self,
        judging_skill: u8,
        observation_count: u8,
        region_penalty: f32,
        is_match: bool,
    ) -> i32 {
        let base = (self.error.max_judging - judging_skill as i16).max(self.error.min_error) as f32;
        let with_region = base * region_penalty;
        let with_context = if is_match {
            with_region * self.error.match_multiplier
        } else {
            with_region
        };
        let sqrt_count = ((observation_count as f32) + 1.0).sqrt();
        (with_context / sqrt_count) as i32
    }

    /// Region penalty applied to the base error term. Domestic
    /// observations get 1.0; foreign observations are scaled by
    /// `(known/unknown) - familiarity/divisor`, floored.
    pub fn region_penalty(&self, is_domestic: bool, is_known_region: bool, familiarity: u8) -> f32 {
        if is_domestic {
            return self.region.domestic_penalty;
        }
        let base = if is_known_region {
            self.region.known_foreign_penalty
        } else {
            self.region.unknown_foreign_penalty
        };
        (base - familiarity as f32 / self.region.familiarity_divisor).max(self.region.penalty_floor)
    }

    /// Youth bonus added to assessed ability before computing the
    /// recommendation tier. Returns 0 if the player isn't young enough
    /// or the potential gap isn't wide enough.
    pub fn youth_bonus(&self, age: u8, assessed_ability: u8, assessed_potential: u8) -> i16 {
        let r = &self.recommendation;
        let gap = assessed_potential.saturating_sub(assessed_ability);
        if age <= r.youth_tier1_age_max && gap > r.youth_tier1_potential_gap_min {
            r.youth_tier1_bonus
        } else if age <= r.youth_tier2_age_max && gap > r.youth_tier2_potential_gap_min {
            r.youth_tier2_bonus
        } else {
            0
        }
    }

    /// Recent-form bonus added to assessed ability before recommendation.
    /// Goes to zero unless the player has both played a meaningful number
    /// of games AND rated well.
    pub fn stats_bonus(&self, appearances: u16, average_rating: f32) -> i16 {
        let r = &self.recommendation;
        if appearances >= r.stats_tier1_apps_min && average_rating >= r.stats_tier1_rating_min {
            r.stats_tier1_bonus
        } else if appearances >= r.stats_tier2_apps_min
            && average_rating >= r.stats_tier2_rating_min
        {
            r.stats_tier2_bonus
        } else {
            0
        }
    }

    /// Adjustment to the assessed ability based on visible recent form.
    /// Distinct from `stats_bonus`: this fires during observation, not
    /// recommendation. Players visibly performing get a small upward
    /// nudge; visibly struggling players a small downward nudge.
    pub fn performance_bonus(&self, appearances: u16, average_rating: f32) -> i32 {
        let r = &self.recommendation;
        if appearances >= r.perf_high_apps_min && average_rating > r.perf_high_rating_min {
            r.perf_high_bonus
        } else if appearances >= r.perf_med_apps_min && average_rating > r.perf_med_rating_min {
            r.perf_med_bonus
        } else if average_rating > 0.0 && average_rating < r.perf_poor_rating_max {
            r.perf_poor_bonus
        } else {
            0
        }
    }

    /// Match-rating-driven bonus to assessed ability for a single match.
    /// Distinct from `performance_bonus` (which uses season aggregates).
    pub fn match_rating_bonus(&self, match_rating: f32) -> i32 {
        let r = &self.recommendation;
        if match_rating > r.match_rating_excellent {
            r.match_rating_excellent_bonus
        } else if match_rating > r.match_rating_good {
            r.match_rating_good_bonus
        } else if match_rating > r.match_rating_ok {
            r.match_rating_ok_bonus
        } else if match_rating < r.match_rating_poor_max {
            r.match_rating_poor_bonus
        } else {
            0
        }
    }

    /// Final recommendation given an effective ability score. The caller
    /// is responsible for adding youth/stats bonuses to the raw assessed
    /// ability before passing it in (use `youth_bonus` + `stats_bonus`).
    pub fn recommendation_for(
        &self,
        effective_ability: i16,
        assessed_ability: u8,
        assessed_potential: u8,
        min_ability: u8,
    ) -> ScoutingRecommendation {
        let r = &self.recommendation;
        let min = min_ability as i16;
        if effective_ability >= min + r.strong_buy_gap
            && assessed_potential > assessed_ability + r.strong_buy_potential_gap
        {
            ScoutingRecommendation::StrongBuy
        } else if effective_ability >= min + r.buy_gap && assessed_potential >= assessed_ability {
            ScoutingRecommendation::Buy
        } else if effective_ability >= min - r.consider_gap {
            ScoutingRecommendation::Consider
        } else {
            ScoutingRecommendation::Pass
        }
    }

    /// Build the risk-flag list for a scouted player using the configured
    /// thresholds. Equivalent to the inline body that used to live in
    /// `helpers::evaluate_risk_flags` — kept there for ABI but delegating
    /// here for the actual policy.
    pub fn risk_flags_for(
        &self,
        is_injured: bool,
        determination: f32,
        age: u8,
        contract_months_remaining: i16,
        player_world_rep: i16,
        buyer_world_rep: i16,
    ) -> Vec<ReportRiskFlag> {
        let r = &self.risk_flags;
        let mut flags = Vec::new();
        if is_injured {
            flags.push(ReportRiskFlag::CurrentlyInjured);
        }
        if determination < r.poor_attitude_determination_max {
            flags.push(ReportRiskFlag::PoorAttitude);
        }
        if age >= r.age_risk_min {
            flags.push(ReportRiskFlag::AgeRisk);
        }
        if contract_months_remaining > 0
            && contract_months_remaining <= r.contract_expiring_months_max
        {
            flags.push(ReportRiskFlag::ContractExpiring);
        }
        if player_world_rep > buyer_world_rep + r.wage_demands_rep_gap {
            flags.push(ReportRiskFlag::WageDemands);
        }
        flags
    }

    /// Target pool size for the data-department pre-filter.
    /// Returns `None` if the candidate set is too small to bother filtering.
    pub fn data_prefilter_target(&self, candidate_count: usize, data_skill: u8) -> Option<usize> {
        let d = &self.data_prefilter;
        if candidate_count <= d.min_pool_size {
            return None;
        }
        let target = d
            .pool_size_at_zero_skill
            .saturating_sub(data_skill as usize * d.pool_size_per_skill_step)
            .max(d.pool_size_floor);
        if candidate_count > target {
            Some(target)
        } else {
            None
        }
    }

    /// Random jitter width when ranking candidates by data score.
    pub fn data_prefilter_noise(&self, data_skill: u8) -> i32 {
        (self.data_prefilter.max_data_skill - data_skill as i32).max(1)
    }

    /// Confidence to record on a freshly-generated pool-context report.
    /// Below the threshold uses `single_observation_confidence`; above it
    /// the standard `1 - 1/(n+1)` curve.
    pub fn pool_report_confidence(&self, observation_count: u8) -> f32 {
        if observation_count <= 1 {
            self.assignment.single_observation_confidence
        } else {
            1.0 - 1.0 / (observation_count as f32 + 1.0)
        }
    }

    /// Confidence to record on a freshly-generated match-context report.
    pub fn match_report_confidence(&self, observation_count: u8) -> f32 {
        (1.0 - 0.5 / (observation_count as f32 + 1.0)).min(1.0)
    }

    /// Decay applied to a shadow report when seeding a new assignment.
    pub fn seeded_shadow_confidence(&self, prior_confidence: f32) -> f32 {
        let s = &self.shadow;
        (prior_confidence * s.seed_confidence_decay)
            .clamp(s.seed_confidence_floor, s.seed_confidence_ceiling)
    }

    /// Decide whether a target is realistic for a buyer at the given
    /// reputation tier.
    ///
    /// We only ever gate scouting **up** (a much bigger selling club); peer
    /// and smaller clubs are unrestricted. When scouting up, a smaller club
    /// can realistically pursue the player only when BOTH:
    ///   * it can **afford** him — salary and value within its tier
    ///     (`target_affordable_for_buyer`), AND
    ///   * he is **attainable** from the bigger club, via one of:
    ///       1. an explicit listing / loan-listing, within a widened tier band
    ///          (`listed_exemption_gap_blocking`) — even a player on the market
    ///          will not drop several tiers;
    ///       2. a backup / not-needed squad role (surplus to the big club);
    ///       3. a fringe playing-time profile (few career games);
    ///       4. a young development prospect not yet individually prominent;
    ///       5. a contract running down (free-transfer pickup).
    /// A first-team regular or key player the bigger club is keeping is out of
    /// reach regardless of budget.
    ///
    /// This encodes the realistic rule: a lower club only chases a top club's
    /// player when he fits its budget AND is surplus there — it is not in the
    /// market for the giant's first-choice keeper.
    /// `buyer_fee_capacity` is the buyer's real fee headroom (transfer budget ×
    /// the negotiation fee-gate multiplier), or `0.0` when the caller has no
    /// budget to hand (a pure reputation-only, aspirational check). A club that
    /// can genuinely FUND a target is allowed to scout him even if his value
    /// sits above the bare reputation proxy, so this gate no longer excludes
    /// players the negotiation layer would happily let a well-funded club bid
    /// for.
    pub fn is_target_realistic(
        &self,
        buyer_world_rep: i16,
        target: &PlayerSummary,
        buyer_fee_capacity: f64,
    ) -> bool {
        self.is_target_realistic_fields(
            buyer_world_rep,
            &RealismTarget {
                club_world_reputation: target.club_world_reputation,
                world_reputation: target.world_reputation,
                current_reputation: target.current_reputation,
                home_reputation: target.home_reputation,
                appearances: target.appearances,
                age: target.age,
                contract_months_remaining: target.contract_months_remaining,
                salary: target.salary,
                estimated_value: target.estimated_value,
                is_listed: target.is_listed,
                is_loan_listed: target.is_loan_listed,
                squad_status: target.seller_ctx.squad_status.clone(),
            },
            buyer_fee_capacity,
        )
    }

    /// Field-level core of [`Self::is_target_realistic`], shared with the
    /// match-scouting path (which reasons about a raw `Player`, not a
    /// `PlayerSummary`). Same policy documented on that method.
    pub fn is_target_realistic_fields(
        &self,
        buyer_world_rep: i16,
        target: &RealismTarget,
        buyer_fee_capacity: f64,
    ) -> bool {
        let r = &self.realism;

        // Only gate scouting "up": a peer or smaller selling club places no
        // restriction on who a buyer may track.
        let club_too_big = target.club_world_reputation > buyer_world_rep + r.club_rep_gap_blocking;
        if !club_too_big {
            return true;
        }

        // Scouting up at a much bigger club. The budget gate is universal
        // here: a smaller side cannot fund a giant's player — fee or wages —
        // whatever his squad role, so an unaffordable target is never
        // realistic. (The user-visible rule: "fits its budget".) A club with
        // real money to spend clears this on its actual budget, not just its
        // reputation tier.
        if !self.target_affordable_for_buyer(buyer_world_rep, target, buyer_fee_capacity) {
            return false;
        }

        // Affordable — but he must also be *attainable* from the bigger club.

        // 1. Explicitly on the market, within a sane tier band (even a listed
        //    player will not drop several divisions).
        if (target.is_listed || target.is_loan_listed)
            && target.club_world_reputation <= buyer_world_rep + r.listed_exemption_gap_blocking
        {
            return true;
        }

        // 2. Surplus by squad role — a backup or unwanted player. (The
        //    user-visible rule: "backup / not-needed status".)
        if target.is_squad_surplus() {
            return true;
        }

        // 3. Fringe by playing time — few career games reads as a reserve /
        //    backup profile even if the contract status wasn't set to one.
        if target.appearances < r.first_team_regular_apps {
            return true;
        }

        // 4. Young development prospect not yet an individually prominent
        //    name. Prominence is judged on EFFECTIVE reputation (a recognised
        //    name in his own market is out of reach even with a modest world
        //    profile); `max(world, blend)` only ever raises the bar.
        let effective_rep = EffectivePlayerReputation::compute(
            target.world_reputation,
            target.current_reputation,
            target.home_reputation,
            true,
        )
        .max(target.world_reputation);
        let player_too_prominent = effective_rep > buyer_world_rep + r.player_rep_gap_blocking;
        if target.age <= r.youth_exempt_age_max && !player_too_prominent {
            return true;
        }

        // 5. Contract running down → a free-transfer pickup, within the same
        //    tier band as a listing. An expiring first-teamer at a giant club
        //    drops at most a couple of tiers on a free, not to the 4th tier.
        if target.contract_months_remaining > 0
            && target.contract_months_remaining <= r.near_free_contract_months
            && target.club_world_reputation <= buyer_world_rep + r.listed_exemption_gap_blocking
        {
            return true;
        }

        // Otherwise: a first-team regular the bigger club is keeping → out of
        // reach for a smaller club regardless of budget.
        false
    }

    /// Linear-tier affordability check — the universal budget gate for
    /// scouting "up". The buyer's reputation sets a salary and value ceiling;
    /// the target must fit both. A tiny club cannot fund a giant's player's
    /// fee or wages, whatever his squad role.
    ///
    /// `buyer_fee_capacity` is the buyer's REAL fee headroom (its transfer
    /// budget × the negotiation fee-gate multiplier), or `0.0` when the caller
    /// has no budget to hand. A target the club can actually FUND is affordable
    /// even if his value sits above the bare reputation proxy — this reconciles
    /// the scouting gate with the negotiation's budget-based fee gate, so a
    /// cash-rich but modest-reputation club (a takeover, or one flush after a
    /// big sale) can pursue the bigger targets its money reaches. It only ever
    /// WIDENS the gate — the reputation proxy still admits aspirational
    /// interest, and the downstream wage plausibility gate still vets salary
    /// exactly, so a funded scout can never smuggle an unaffordable wage
    /// through here.
    fn target_affordable_for_buyer(
        &self,
        buyer_world_rep: i16,
        target: &RealismTarget,
        buyer_fee_capacity: f64,
    ) -> bool {
        let r = &self.realism;
        let buyer_tier = buyer_world_rep.max(0) as f64;
        let max_salary = buyer_tier * r.salary_per_rep_point;
        let max_value = buyer_tier * r.value_per_rep_point;
        let rep_affordable =
            (target.salary as f64) <= max_salary && target.estimated_value <= max_value;
        let budget_affordable =
            buyer_fee_capacity > 0.0 && target.estimated_value <= buyer_fee_capacity;
        rep_affordable || budget_affordable
    }

    /// Per-Monday refresh chunk size for shadow reports.
    pub fn shadow_refresh_count(&self, total_shadow_reports: usize) -> usize {
        let s = &self.shadow;
        (total_shadow_reports / s.refresh_count_divisor)
            .max(s.refresh_count_min)
            .min(s.refresh_count_max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PlayerFieldPositionGroup;
    use crate::PlayerPositionType;
    use crate::PlayerSquadStatus;
    use crate::transfers::ScoutingRegion;
    use crate::transfers::pipeline::SellerPlausibilityContext;

    /// Test fixture: builds a `PlayerSummary` with a baseline "anonymous
    /// big-club regular" profile. Each call overrides only the fields the
    /// test cares about. Salary/value default to figures that fit a
    /// mid/lower-tier buyer's affordability check, so tests that don't
    /// specifically probe affordability stay readable.
    struct Target {
        appearances: u16,
        age: u8,
        contract_months: i16,
        is_listed: bool,
        is_loan_listed: bool,
        club_world_rep: i16,
        world_rep: i16,
        salary: u32,
        estimated_value: f64,
        squad_status: PlayerSquadStatus,
    }

    impl Default for Target {
        fn default() -> Self {
            Target {
                appearances: 32,
                age: 28,
                contract_months: 36,
                is_listed: false,
                is_loan_listed: false,
                club_world_rep: 7500,
                world_rep: 4000,
                salary: 500_000,
                estimated_value: 1_500_000.0,
                squad_status: PlayerSquadStatus::FirstTeamRegular,
            }
        }
    }

    impl Target {
        fn build(self) -> PlayerSummary {
            PlayerSummary {
                player_id: 1,
                club_id: 100,
                country_id: 1,
                continent_id: 1,
                region: ScoutingRegion::from_country(1, "RU"),
                country_code: "RU".to_string(),
                player_name: "Test".to_string(),
                club_name: "Test Club".to_string(),
                position: PlayerPositionType::Goalkeeper,
                position_group: PlayerFieldPositionGroup::Goalkeeper,
                age: self.age,
                estimated_value: self.estimated_value,
                is_listed: self.is_listed,
                is_loan_listed: self.is_loan_listed,
                skill_ability: 150,
                average_rating: 7.0,
                goals: 0,
                assists: 0,
                appearances: self.appearances,
                determination: 12.0,
                work_rate: 12.0,
                composure: 12.0,
                anticipation: 12.0,
                technical_avg: 12.0,
                mental_avg: 12.0,
                physical_avg: 12.0,
                current_reputation: self.world_rep,
                home_reputation: self.world_rep,
                world_reputation: self.world_rep,
                country_reputation: 7000,
                club_world_reputation: self.club_world_rep,
                club_best_in_group: 150,
                is_injured: false,
                contract_months_remaining: self.contract_months,
                salary: self.salary,
                seller_ctx: SellerPlausibilityContext {
                    club_reputation_score: (self.club_world_rep.max(0) as f32 / 10000.0),
                    league_reputation: 5500,
                    league_id: None,
                    position_group_rank: 0,
                    squad_status: self.squad_status.clone(),
                    is_transfer_requested: false,
                    is_unhappy: false,
                    in_debt: false,
                },
            }
        }
    }

    #[test]
    fn observation_chance_scales_with_skill() {
        let c = ScoutingConfig::default();
        assert_eq!(c.daily_observation_chance(0), 60);
        assert_eq!(c.daily_observation_chance(20), 70);
    }

    #[test]
    fn observations_per_day_in_2_to_4_range() {
        let c = ScoutingConfig::default();
        assert_eq!(c.observations_per_day(0), 2);
        assert_eq!(c.observations_per_day(15), 3);
        assert_eq!(c.observations_per_day(20), 4);
    }

    #[test]
    fn region_penalty_falls_with_familiarity() {
        let c = ScoutingConfig::default();
        let cold = c.region_penalty(false, false, 0);
        let warm = c.region_penalty(false, false, 100);
        assert!(warm < cold);
        assert!(warm >= c.region.penalty_floor);
    }

    #[test]
    fn region_penalty_floor_is_respected() {
        let c = ScoutingConfig::default();
        // Even at familiarity=200 (off the chart) we must not drop below floor.
        let p = c.region_penalty(false, true, 200);
        assert!(p >= c.region.penalty_floor);
    }

    #[test]
    fn error_decreases_with_observation_count() {
        let c = ScoutingConfig::default();
        let e1 = c.effective_error(10, 0, 1.0, false);
        let e10 = c.effective_error(10, 9, 1.0, false);
        assert!(e10 < e1);
    }

    #[test]
    fn match_context_yields_lower_error_than_pool() {
        let c = ScoutingConfig::default();
        let pool = c.effective_error(10, 1, 1.0, false);
        let match_ = c.effective_error(10, 1, 1.0, true);
        assert!(match_ < pool);
    }

    #[test]
    fn youth_bonus_only_fires_for_young_with_gap() {
        let c = ScoutingConfig::default();
        // Young + big gap → tier 1
        assert_eq!(
            c.youth_bonus(20, 100, 130),
            c.recommendation.youth_tier1_bonus
        );
        // Young + medium gap → tier 2
        assert_eq!(
            c.youth_bonus(22, 100, 115),
            c.recommendation.youth_tier2_bonus
        );
        // Old + gap → no bonus
        assert_eq!(c.youth_bonus(28, 100, 130), 0);
        // Young + no gap → no bonus
        assert_eq!(c.youth_bonus(20, 100, 105), 0);
    }

    #[test]
    fn recommendation_tiers() {
        let c = ScoutingConfig::default();
        // StrongBuy: well above min, with potential gap
        let r = c.recommendation_for(120, 100, 110, 105);
        assert_eq!(r, ScoutingRecommendation::StrongBuy);
        // Buy: meets min, potential ≥ ability
        let r = c.recommendation_for(105, 100, 100, 105);
        assert_eq!(r, ScoutingRecommendation::Buy);
        // Consider: just below min
        let r = c.recommendation_for(102, 100, 100, 105);
        assert_eq!(r, ScoutingRecommendation::Consider);
        // Pass: well below min
        let r = c.recommendation_for(50, 50, 50, 105);
        assert_eq!(r, ScoutingRecommendation::Pass);
    }

    #[test]
    fn risk_flags_fire_on_thresholds() {
        let c = ScoutingConfig::default();
        let flags = c.risk_flags_for(true, 5.0, 32, 4, 8000, 5000);
        assert!(flags.contains(&ReportRiskFlag::CurrentlyInjured));
        assert!(flags.contains(&ReportRiskFlag::PoorAttitude));
        assert!(flags.contains(&ReportRiskFlag::AgeRisk));
        assert!(flags.contains(&ReportRiskFlag::ContractExpiring));
        assert!(flags.contains(&ReportRiskFlag::WageDemands));
    }

    #[test]
    fn risk_flags_quiet_on_safe_player() {
        let c = ScoutingConfig::default();
        let flags = c.risk_flags_for(false, 15.0, 25, 24, 5000, 5000);
        assert!(flags.is_empty());
    }

    #[test]
    fn data_prefilter_skips_small_pools() {
        let c = ScoutingConfig::default();
        assert!(c.data_prefilter_target(15, 10).is_none());
    }

    #[test]
    fn data_prefilter_narrows_with_skill() {
        let c = ScoutingConfig::default();
        let lo = c.data_prefilter_target(200, 0).unwrap();
        let hi = c.data_prefilter_target(200, 20).unwrap();
        assert!(hi < lo);
        assert!(hi >= c.data_prefilter.pool_size_floor);
    }

    #[test]
    fn pool_report_confidence_grows_with_observations() {
        let c = ScoutingConfig::default();
        let one = c.pool_report_confidence(1);
        let ten = c.pool_report_confidence(10);
        assert!(ten > one);
        assert!(ten <= 1.0);
    }

    #[test]
    fn seeded_shadow_confidence_decays_and_clamps() {
        let c = ScoutingConfig::default();
        let decayed = c.seeded_shadow_confidence(0.9);
        assert!(decayed < 0.9);
        // Floor: even a near-zero prior must not go below the floor.
        let floored = c.seeded_shadow_confidence(0.01);
        assert!(floored >= c.shadow.seed_confidence_floor);
    }

    #[test]
    fn shadow_refresh_count_respects_bounds() {
        let c = ScoutingConfig::default();
        assert_eq!(c.shadow_refresh_count(0), c.shadow.refresh_count_min);
        assert_eq!(c.shadow_refresh_count(100), c.shadow.refresh_count_max);
    }

    // ============================================================
    // Realism gate
    // ============================================================
    //
    // Buyer rep ~3500 stands in for a mid/lower-tier domestic club;
    // ~7500 stands in for a top-flight Russian club like CSKA / Zenit.
    // Reputation scale is 0-10000.

    const LOWER_TIER_BUYER: i16 = 3500;
    const TOP_TIER_BUYER: i16 = 7500;
    /// A 3rd/4th-tier side (Strogino-ish) — several reputation tiers below a
    /// top-flight seller. Used to verify availability does not erase the gap.
    const FOURTH_TIER_BUYER: i16 = 1500;

    #[test]
    fn realism_blocks_top_club_first_team_regular_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        // CSKA-ish #1 GK: 30+ apps, 28y, long contract, prominent rep.
        let star_gk = Target {
            world_rep: 6500,
            ..Target::default()
        }
        .build();
        assert!(!c.is_target_realistic(LOWER_TIER_BUYER, &star_gk, 0.0));
    }

    #[test]
    fn realism_blocks_high_home_rep_domestic_star_with_low_world_rep() {
        let c = ScoutingConfig::default();
        // A domestic great: modest world reputation, big home / current
        // standing, a young first-team regular at a much bigger club. On
        // bare world rep (2869) he clears the prominence gap for a
        // lower-tier buyer (3500 + 1500 = 5000) and the youth exemption
        // would wave him through — but his EFFECTIVE reputation (~5200,
        // weighting his domestic renown) does not, so he stays unrealistic.
        let mut star = Target {
            world_rep: 2869,
            age: 20,
            appearances: 25,
            ..Target::default()
        }
        .build();
        star.current_reputation = 5575;
        star.home_reputation = 6177;
        assert!(
            !c.is_target_realistic(LOWER_TIER_BUYER, &star, 0.0),
            "a domestic great must not read as a low-rep bargain in his market"
        );

        // Control: collapse home / current down to the low world level — now
        // he is genuinely low-reputation and the youth exemption applies.
        let mut anonymous = star.clone();
        anonymous.current_reputation = 2869;
        anonymous.home_reputation = 2869;
        assert!(
            c.is_target_realistic(LOWER_TIER_BUYER, &anonymous, 0.0),
            "with world-level home/current rep the young regular is reachable"
        );
    }

    #[test]
    fn realism_allows_loan_listed_top_club_player_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        // Same star, but the parent club has put him on the loan list.
        let loan_listed = Target {
            world_rep: 6500,
            is_loan_listed: true,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &loan_listed, 0.0));
    }

    #[test]
    fn realism_allows_transfer_listed_top_club_player_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        let listed = Target {
            world_rep: 6500,
            is_listed: true,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &listed, 0.0));
    }

    #[test]
    fn realism_blocks_loan_listed_top_club_regular_for_vastly_lower_buyer() {
        let c = ScoutingConfig::default();
        // The Pichienko case: a loan-listed first-choice keeper at a giant
        // club (7500). A mid-tier club may pursue a loan, but a side several
        // tiers below (1500) cannot realistically host / pay / attract him —
        // availability does not erase the level gap, so the listing exemption
        // no longer waves him through and the prominence block catches him.
        let loan_listed_regular = Target {
            world_rep: 6000,
            is_loan_listed: true,
            ..Target::default() // club_world_rep 7500, appearances 32 (regular)
        }
        .build();
        assert!(
            !c.is_target_realistic(FOURTH_TIER_BUYER, &loan_listed_regular, 0.0),
            "a 4th-tier side must not scout a loan-listed top-club first-teamer"
        );
        // Control: a mid/lower-tier club (within the widened band) still can —
        // a plausible loan destination is not over-blocked.
        assert!(
            c.is_target_realistic(LOWER_TIER_BUYER, &loan_listed_regular, 0.0),
            "a mid-tier club may still pursue the loan-listed player"
        );
    }

    #[test]
    fn realism_blocks_transfer_listed_top_club_regular_for_vastly_lower_buyer() {
        let c = ScoutingConfig::default();
        // Same level gap, permanent listing: a transfer-listed prominent
        // first-teamer at a giant club is still out of a 4th-tier side's reach.
        let listed_regular = Target {
            world_rep: 6000,
            is_listed: true,
            ..Target::default()
        }
        .build();
        assert!(!c.is_target_realistic(FOURTH_TIER_BUYER, &listed_regular, 0.0));
    }

    #[test]
    fn realism_still_allows_listed_low_rep_youth_for_vastly_lower_buyer() {
        let c = ScoutingConfig::default();
        // Beyond the listed band the gate falls through to the normal layers
        // — which must still pass a genuine surplus youngster (few career
        // games, low rep) the big club has loan-listed for development.
        let listed_youth = Target {
            appearances: 4,
            age: 19,
            world_rep: 2200,
            is_loan_listed: true,
            ..Target::default()
        }
        .build();
        assert!(
            c.is_target_realistic(FOURTH_TIER_BUYER, &listed_youth, 0.0),
            "a surplus loan-listed youngster stays reachable via the fringe path"
        );
    }

    // ── The two-condition rule: a lower club may pursue a top club's player
    //    only when he both FITS ITS BUDGET and is a BACKUP / NOT-NEEDED squad
    //    member. ──

    #[test]
    fn realism_allows_affordable_backup_at_top_club_for_lower_buyer() {
        let c = ScoutingConfig::default();
        // A declared backup at a giant club — an established keeper by minutes
        // but explicitly second choice — on wages/value a mid-tier club can
        // fund. Reachable: surplus + affordable.
        let backup = Target {
            appearances: 20,                                   // a regular by minutes…
            squad_status: PlayerSquadStatus::MainBackupPlayer, // …but a backup by role
            salary: 500_000,
            estimated_value: 1_500_000.0,
            world_rep: 4000,
            ..Target::default()
        }
        .build();
        assert!(
            c.is_target_realistic(LOWER_TIER_BUYER, &backup, 0.0),
            "an affordable backup at a top club is a realistic lower-club target"
        );
    }

    #[test]
    fn realism_blocks_unaffordable_backup_at_top_club_for_lower_buyer() {
        let c = ScoutingConfig::default();
        // Same backup role, but on top-club wages a tier-3 club can't fund.
        // Budget condition fails → not realistic even though he is surplus.
        let pricey_backup = Target {
            appearances: 20,
            squad_status: PlayerSquadStatus::MainBackupPlayer,
            salary: 5_000_000,
            estimated_value: 15_000_000.0,
            world_rep: 4000,
            ..Target::default()
        }
        .build();
        assert!(
            !c.is_target_realistic(LOWER_TIER_BUYER, &pricey_backup, 0.0),
            "a backup the buyer can't afford is not a realistic target"
        );
    }

    #[test]
    fn realism_blocks_affordable_first_team_regular_at_top_club_for_lower_buyer() {
        let c = ScoutingConfig::default();
        // A first-team regular at a giant club, even on cheap terms, is not in
        // the market for a lower club — he is not surplus. Role condition fails.
        let regular = Target {
            appearances: 30,
            squad_status: PlayerSquadStatus::FirstTeamRegular,
            salary: 500_000,
            estimated_value: 1_500_000.0,
            world_rep: 4000,
            age: 27,
            ..Target::default()
        }
        .build();
        assert!(
            !c.is_target_realistic(LOWER_TIER_BUYER, &regular, 0.0),
            "an affordable first-team regular is still out of a lower club's reach"
        );
    }

    #[test]
    fn realism_allows_top_club_fringe_backup_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        // Backup keeper at a big club — very few apps.
        let backup = Target {
            appearances: 3,
            age: 26,
            world_rep: 5000,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &backup, 0.0));
    }

    #[test]
    fn realism_allows_low_rep_top_club_youth_reserve_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        // 19-y-o academy product, low world reputation, few first-team
        // apps. Big clubs sell their surplus youth to smaller clubs.
        let youth_reserve = Target {
            appearances: 5,
            age: 19,
            world_rep: 2500,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &youth_reserve, 0.0));
    }

    #[test]
    fn realism_allows_moderate_rep_top_club_youth_regular_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        // 20-y-o getting first-team minutes at a big club but not yet a
        // headline name (moderate world rep) — soft youth exemption.
        let youth_regular = Target {
            appearances: 22,
            age: 20,
            world_rep: 4500,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &youth_regular, 0.0));
    }

    #[test]
    fn realism_blocks_high_rep_top_club_youth_star_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        // 21-y-o first-team STAR at a giant club — wages and standing
        // already out of reach for a tier-3 buyer. Youth no longer
        // bypasses the prominence gate.
        let youth_star = Target {
            appearances: 30,
            age: 21,
            world_rep: 6000,
            ..Target::default()
        }
        .build();
        assert!(!c.is_target_realistic(LOWER_TIER_BUYER, &youth_star, 0.0));
    }

    #[test]
    fn realism_blocks_expiring_elite_for_lower_tier_buyer() {
        let c = ScoutingConfig::default();
        // World-rep clears the prominence gap → hard block, expiring
        // contract cannot rescue.
        let expiring_star = Target {
            contract_months: 8,
            world_rep: 6500,
            salary: 5_000_000,
            estimated_value: 15_000_000.0,
            ..Target::default()
        }
        .build();
        assert!(!c.is_target_realistic(LOWER_TIER_BUYER, &expiring_star, 0.0));
    }

    #[test]
    fn realism_blocks_expiring_player_with_unaffordable_wages() {
        let c = ScoutingConfig::default();
        // World-rep stays under the prominence gap (4500 vs 3500+1500=5000)
        // so the prominence layer doesn't fire — but the salary is still
        // top-club money. Affordability check inside the expiring exemption
        // catches this.
        let expiring_high_wage = Target {
            contract_months: 8,
            world_rep: 4500,
            salary: 5_000_000,
            estimated_value: 4_000_000.0,
            ..Target::default()
        }
        .build();
        assert!(!c.is_target_realistic(LOWER_TIER_BUYER, &expiring_high_wage, 0.0));
    }

    #[test]
    fn realism_allows_expiring_player_with_realistic_wages() {
        let c = ScoutingConfig::default();
        // Sub-prominent first-team regular at a much bigger club, contract
        // running down, wages and value within the buyer's tier. This is
        // the canonical free-transfer pickup that should remain attainable.
        let expiring_realistic = Target {
            contract_months: 8,
            world_rep: 4000,
            salary: 600_000,
            estimated_value: 1_200_000.0,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &expiring_realistic, 0.0));
    }

    #[test]
    fn realism_allows_similar_tier_clubs_to_scout_each_other() {
        let c = ScoutingConfig::default();
        // Two top-flight clubs with similar reputation — Spartak scouting
        // CSKA. Both ends in the 7000-8000 band.
        let peer_target = Target {
            club_world_rep: 7800,
            world_rep: 6500,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(TOP_TIER_BUYER, &peer_target, 0.0));
    }

    #[test]
    fn realism_allows_top_club_to_scout_smaller_club_regular() {
        let c = ScoutingConfig::default();
        // Reverse direction — Zenit scouting a lower-tier first-team GK.
        // Cap is one-sided (only blocks scouting "up").
        let smaller_club_regular = Target {
            club_world_rep: 3500,
            world_rep: 3500,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(TOP_TIER_BUYER, &smaller_club_regular, 0.0));
    }

    #[test]
    fn realism_layers_short_circuit_in_order() {
        let c = ScoutingConfig::default();
        // Listed bypasses everything, even an otherwise-blocked elite.
        let listed_elite = Target {
            world_rep: 6500,
            is_listed: true,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &listed_elite, 0.0));

        // Same prominent profile but at a peer-sized club: club gap doesn't
        // fire, so we never reach the prominence block.
        let peer_prominent = Target {
            club_world_rep: 7800,
            world_rep: 6500,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(TOP_TIER_BUYER, &peer_prominent, 0.0));

        // Big-club elite played only twice all season → fringe path.
        let big_both_fringe = Target {
            appearances: 2,
            world_rep: 6500,
            ..Target::default()
        }
        .build();
        assert!(c.is_target_realistic(LOWER_TIER_BUYER, &big_both_fringe, 0.0));
    }
}
