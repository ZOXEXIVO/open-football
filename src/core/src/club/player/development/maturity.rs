//! Maturity-aware development helpers — the "right challenge, right dose,
//! right age" model.
//!
//! Grouped into [`MaturityModel`], a zero-state namespace whose associated
//! functions encode each slice of the model:
//!
//! * [`MaturityModel::biological_maturity_multiplier`] — physiological /
//!   cognitive ceiling on category growth at a given age.
//! * [`MaturityModel::senior_exposure_multiplier`] — turns rolling 30-day
//!   minutes and physical load into the development signal that match
//!   exposure should provide, with a per-age optimal band and steep
//!   penalty for overuse.
//! * [`MaturityModel::overload_development_modifier`] — acute workload
//!   state (condition, jadedness, recovery debt, recent load).
//! * [`MaturityModel::weekly_growth_cap`] — hard per-skill, per-week
//!   ceiling applied *after* every multiplier stacks. Stops compounding
//!   bonuses from producing implausible jumps.
//! * [`MaturityModel::challenge_band_multiplier`] — gates exposure by
//!   whether the player is plausibly competing at the level. An under-19
//!   dropped into a league far above his CA learns less, not more.
//! * [`MaturityModel::step_up_age_factor`] — fraction of the post-transfer
//!   step-up bonus a player is allowed to absorb at this age.
//!
//! All methods are pure functions of their inputs; the struct exists for
//! namespacing/discoverability rather than to carry state. Callers go
//! through `MaturityModel::xxx(...)` so a future move to a configurable
//! version (per-difficulty tuning) only changes the call site to take
//! `&self`.

use super::skills_array::SkillCategory;

/// Zero-state grouping of maturity-aware development helpers. See module
/// docs for the model.
pub(super) struct MaturityModel;

impl MaturityModel {
    /// Biological/cognitive maturity dampener applied to growth rates from
    /// the raw age curve. Returns a multiplier in `[0.0, 1.0]`.
    ///
    /// Football model:
    ///   * Physical growth (strength, stamina, jumping, natural fitness)
    ///     tracks skeletal/muscular maturation. Under 16 it is gated by
    ///     puberty timing — minutes alone don't add strength.
    ///   * Pace and acceleration mature earlier than strength but still cap.
    ///   * Technical motor patterns can be drilled at any age, but the
    ///     neuromuscular myelination needed to make them automatic under
    ///     senior pressure isn't fully online until late adolescence.
    ///   * Mental skills (decisions, composure, leadership) lag furthest —
    ///     senior football alone barely shifts a 14-year-old's
    ///     decision-making.
    pub(super) fn biological_maturity_multiplier(age: u8, cat: SkillCategory) -> f32 {
        match cat {
            SkillCategory::Physical => match age {
                0..=13 => 0.20,
                14 => 0.30,
                15 => 0.45,
                16 => 0.65,
                17 => 0.82,
                18 => 0.94,
                _ => 1.0,
            },
            SkillCategory::Technical => match age {
                0..=13 => 0.35,
                14 => 0.45,
                15 => 0.58,
                16 => 0.78,
                17 => 0.90,
                _ => 1.0,
            },
            SkillCategory::Mental => match age {
                0..=13 => 0.25,
                14 => 0.35,
                15 => 0.48,
                16 => 0.68,
                17 => 0.86,
                _ => 1.0,
            },
            SkillCategory::Goalkeeping => match age {
                0..=13 => 0.25,
                14 => 0.40,
                15 => 0.55,
                16 => 0.72,
                17 => 0.88,
                _ => 1.0,
            },
        }
    }

    /// Senior-exposure multiplier. Returns a value in roughly
    /// `[0.45, 1.30]` that scales match-driven development based on
    /// rolling minutes and load, not raw appearance counts.
    ///
    /// `minutes_last_30` and `physical_load_30` come from
    /// [`crate::club::player::load::PlayerLoad`]. `league_reputation` is
    /// the 0–10000 league rep and `player_ca` is 0–200.
    ///
    /// Football model — optimal monthly minute bands (per
    /// [`MaturityModel::optimal_minutes_band`]):
    ///   * 14-15: 0–300 useful, 300–600 diminishing, above → outright burn-out
    ///   * 16-17: 300–900 useful, 900–1500 diminishing
    ///   * 18-21: 600–1800 useful, 1800–2500 diminishing
    ///   * 22-29: 600–2200 useful (peak window)
    pub(super) fn senior_exposure_multiplier(
        age: u8,
        minutes_last_30: f32,
        physical_load_30: f32,
        league_reputation: u16,
        player_ca: u8,
    ) -> f32 {
        let (low, high, hard_cap) = Self::optimal_minutes_band(age);
        let m = minutes_last_30.max(0.0);

        let band_score = if m <= low {
            let progress = if low > 0.0 { m / low } else { 1.0 };
            0.95 + progress * 0.15
        } else if m <= high {
            let span = (high - low).max(1.0);
            let progress = (m - low) / span;
            1.10 + progress * 0.10
        } else if m <= hard_cap {
            let span = (hard_cap - high).max(1.0);
            let over = (m - high) / span;
            1.20 - over * 0.65
        } else {
            0.55
        };

        let chronic_tax = Self::chronic_load_tax(age, physical_load_30);
        let challenge_factor = Self::challenge_band_multiplier(age, player_ca, league_reputation);

        (band_score * chronic_tax * challenge_factor).clamp(0.45, 1.30)
    }

    /// Optimal monthly competitive-minute band per age. Returns
    /// `(low, high, hard_cap)` where `[low, high]` is the productive zone
    /// and `hard_cap` is where the multiplier bottoms out at the burn-out
    /// floor.
    fn optimal_minutes_band(age: u8) -> (f32, f32, f32) {
        match age {
            0..=15 => (0.0, 300.0, 600.0),
            16..=17 => (300.0, 900.0, 1500.0),
            18..=21 => (600.0, 1800.0, 2500.0),
            22..=29 => (600.0, 2200.0, 3000.0),
            _ => (300.0, 1800.0, 3000.0),
        }
    }

    /// Chronic load tax — heavy 30-day physical load erodes development
    /// even inside the optimal minute band. Younger players bottom out
    /// earlier.
    fn chronic_load_tax(age: u8, physical_load_30: f32) -> f32 {
        let danger = match age {
            0..=15 => 700.0,
            16..=17 => 1200.0,
            _ => 2000.0,
        };
        let l = physical_load_30.max(0.0);
        if l <= danger {
            1.0
        } else {
            let over = (l - danger) / danger;
            (1.0 - over * 0.35).clamp(0.55, 1.0)
        }
    }

    /// Whether the league rep / player CA pairing is a sensible learning
    /// challenge for an under-19 player. Returns `[0.55, 1.10]`.
    ///
    /// Football model: when a 14yo with CA 60 is dropped into a 9000-rep
    /// league, he survives plays without comprehending them — exposure
    /// does not turn into learning. A young player at a level *below* his
    /// CA (e.g. on loan in a lower division) still trains with peers and
    /// gets a small "big fish" bonus.
    ///
    /// Adults (19+) skip the gate entirely — managers may have any reason
    /// to punch a player above his weight, and senior development isn't
    /// gated by it the way youth development is.
    pub(super) fn challenge_band_multiplier(
        age: u8,
        player_ca: u8,
        league_reputation: u16,
    ) -> f32 {
        if age >= 19 {
            return 1.0;
        }
        let expected_ca = (league_reputation as f32 / 65.0).clamp(40.0, 160.0);
        let gap = expected_ca - player_ca as f32;
        if gap <= 0.0 {
            return 1.05;
        }
        let g = (gap / 40.0).clamp(0.0, 1.0);
        (1.0 - g * 0.40).max(0.55)
    }

    /// Reduces growth (toward zero, never below ~0.4) when the player's
    /// acute workload state is bad: low condition, high jadedness, deep
    /// recovery debt, or a 7-day load above the age-appropriate ceiling.
    /// Under-18s suffer harder under each input — adolescent recovery
    /// curves are not adult curves.
    pub(super) fn overload_development_modifier(
        age: u8,
        physical_load_7: f32,
        condition_pct: u32,
        jadedness: i16,
        recovery_debt: f32,
    ) -> f32 {
        let cond = (condition_pct as f32 / 100.0).clamp(0.0, 1.0);
        let jad = (jadedness.max(0) as f32 / 10000.0).clamp(0.0, 1.0);
        let debt_share = (recovery_debt / 800.0).clamp(0.0, 1.0);

        let load_ceiling = match age {
            0..=15 => 220.0,
            16..=17 => 380.0,
            _ => 520.0,
        };
        let load_pressure = (physical_load_7 / load_ceiling).clamp(0.0, 2.0);

        let young_factor = if age <= 15 {
            1.5
        } else if age <= 17 {
            1.25
        } else {
            1.0
        };

        let mut penalty = 0.0;
        penalty += (1.0 - cond) * 0.30 * young_factor;
        penalty += jad * 0.25 * young_factor;
        penalty += debt_share * 0.20 * young_factor;
        penalty += (load_pressure - 1.0).max(0.0) * 0.30 * young_factor;

        (1.0 - penalty).clamp(0.40, 1.0)
    }

    /// Per-week, per-category hard cap on positive skill change. Applied
    /// after every multiplier has stacked, so no compounding stack of
    /// bonuses can produce a multi-CA-point jump in a single week.
    ///
    /// Tuning rationale: a season of 52 weekly ticks at the 18-19yo
    /// technical cap (0.055) amounts to ~2.8 raw skill points pre-cap,
    /// which is already the upper bound of a realistic year of
    /// development for a top prospect. The 14-15yo physical cap (0.006)
    /// deliberately makes physical CA gains a slow drip — a season at
    /// the cap is ~0.3 raw points of pace.
    pub(super) fn weekly_growth_cap(age: u8, cat: SkillCategory) -> f32 {
        match cat {
            SkillCategory::Physical => match age {
                0..=15 => 0.006,
                16..=17 => 0.018,
                18..=19 => 0.025,
                20..=22 => 0.022,
                23..=27 => 0.015,
                _ => 0.010,
            },
            SkillCategory::Technical => match age {
                0..=15 => 0.018,
                16..=17 => 0.040,
                18..=19 => 0.055,
                20..=22 => 0.045,
                23..=27 => 0.030,
                _ => 0.020,
            },
            SkillCategory::Mental => match age {
                0..=15 => 0.014,
                16..=17 => 0.030,
                18..=19 => 0.040,
                20..=22 => 0.038,
                23..=27 => 0.030,
                _ => 0.022,
            },
            SkillCategory::Goalkeeping => match age {
                0..=15 => 0.010,
                16..=17 => 0.030,
                18..=19 => 0.040,
                20..=22 => 0.038,
                23..=27 => 0.032,
                _ => 0.022,
            },
        }
    }

    /// Fraction of the post-transfer step-up bonus that should pass
    /// through at this age. The step-up multiplier exists to model
    /// players raising their game alongside higher-calibre teammates — a
    /// dynamic that requires the player to actually *be* a peer of the
    /// senior squad. A 14yo at an elite club trains with the U16s; the
    /// multiplier shouldn't fire.
    pub(super) fn step_up_age_factor(age: u8) -> f32 {
        match age {
            0..=15 => 0.0,
            16..=17 => 0.20,
            18 => 0.65,
            _ => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biological_maturity_under_16_dampens_physical() {
        let young = MaturityModel::biological_maturity_multiplier(14, SkillCategory::Physical);
        let prime = MaturityModel::biological_maturity_multiplier(20, SkillCategory::Physical);
        assert!(young < prime * 0.5, "young={} prime={}", young, prime);
    }

    #[test]
    fn biological_maturity_orderings_are_monotone() {
        for cat in [
            SkillCategory::Physical,
            SkillCategory::Technical,
            SkillCategory::Mental,
            SkillCategory::Goalkeeping,
        ] {
            let v14 = MaturityModel::biological_maturity_multiplier(14, cat);
            let v16 = MaturityModel::biological_maturity_multiplier(16, cat);
            let v18 = MaturityModel::biological_maturity_multiplier(18, cat);
            assert!(v14 <= v16 && v16 <= v18, "non-monotone for {:?}", cat);
        }
    }

    #[test]
    fn weekly_growth_cap_under_16_smaller_than_18_19() {
        assert!(
            MaturityModel::weekly_growth_cap(14, SkillCategory::Physical)
                < MaturityModel::weekly_growth_cap(18, SkillCategory::Physical)
        );
        assert!(
            MaturityModel::weekly_growth_cap(15, SkillCategory::Technical)
                < MaturityModel::weekly_growth_cap(19, SkillCategory::Technical)
        );
        assert!(
            MaturityModel::weekly_growth_cap(15, SkillCategory::Mental)
                < MaturityModel::weekly_growth_cap(19, SkillCategory::Mental)
        );
    }

    #[test]
    fn senior_exposure_overloaded_youngster_drops_below_one() {
        let m = MaturityModel::senior_exposure_multiplier(14, 1500.0, 800.0, 8000, 70);
        assert!(m < 1.0, "got {}", m);
    }

    #[test]
    fn senior_exposure_managed_youth_minutes_lift_above_one() {
        let m = MaturityModel::senior_exposure_multiplier(15, 200.0, 200.0, 5000, 80);
        assert!(m > 1.0, "got {}", m);
    }

    #[test]
    fn senior_exposure_extreme_overload_floors_below_band_score() {
        let extreme = MaturityModel::senior_exposure_multiplier(14, 3000.0, 1500.0, 9000, 60);
        let normal = MaturityModel::senior_exposure_multiplier(14, 200.0, 200.0, 5000, 80);
        assert!(
            extreme < normal * 0.65,
            "extreme={} normal={}",
            extreme,
            normal
        );
    }

    #[test]
    fn challenge_band_overmatched_youngster_gets_dampener() {
        let m = MaturityModel::challenge_band_multiplier(14, 60, 9000);
        assert!(m < 0.95, "got {}", m);
    }

    #[test]
    fn challenge_band_adult_unaffected() {
        assert_eq!(MaturityModel::challenge_band_multiplier(22, 60, 9000), 1.0);
    }

    #[test]
    fn challenge_band_youth_at_level_neutral_or_above() {
        let m = MaturityModel::challenge_band_multiplier(17, 130, 5000);
        assert!(m >= 1.0);
    }

    #[test]
    fn overload_modifier_drained_youngster_drops_far() {
        let m = MaturityModel::overload_development_modifier(15, 600.0, 35, 8500, 800.0);
        assert!(m < 0.7, "got {}", m);
    }

    #[test]
    fn overload_modifier_fresh_player_neutral() {
        let m = MaturityModel::overload_development_modifier(19, 100.0, 95, 500, 80.0);
        assert!(m > 0.95, "got {}", m);
    }

    #[test]
    fn overload_modifier_under_15_more_punitive_than_adult() {
        let young = MaturityModel::overload_development_modifier(15, 400.0, 40, 7000, 600.0);
        let adult = MaturityModel::overload_development_modifier(22, 400.0, 40, 7000, 600.0);
        assert!(young < adult, "young={} adult={}", young, adult);
    }

    #[test]
    fn step_up_age_factor_under_15_strips_bonus() {
        assert_eq!(MaturityModel::step_up_age_factor(14), 0.0);
        assert!(MaturityModel::step_up_age_factor(17) <= 0.25);
        assert_eq!(MaturityModel::step_up_age_factor(20), 1.0);
    }
}
