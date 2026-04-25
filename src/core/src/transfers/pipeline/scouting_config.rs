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

use crate::transfers::pipeline::ScoutingRecommendation;
use crate::transfers::ReportRiskFlag;

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
    pub fn region_penalty(
        &self,
        is_domestic: bool,
        is_known_region: bool,
        familiarity: u8,
    ) -> f32 {
        if is_domestic {
            return self.region.domestic_penalty;
        }
        let base = if is_known_region {
            self.region.known_foreign_penalty
        } else {
            self.region.unknown_foreign_penalty
        };
        (base - familiarity as f32 / self.region.familiarity_divisor)
            .max(self.region.penalty_floor)
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
    pub fn performance_bonus(
        &self,
        appearances: u16,
        average_rating: f32,
    ) -> i32 {
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
        assert_eq!(c.youth_bonus(20, 100, 130), c.recommendation.youth_tier1_bonus);
        // Young + medium gap → tier 2
        assert_eq!(c.youth_bonus(22, 100, 115), c.recommendation.youth_tier2_bonus);
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
}
