//! Unified injury-risk recipe shared across spontaneous, training,
//! match, and recovery-setback paths.
//!
//! Previously each path computed risk inline with slightly different
//! shapes — match used minutes + jadedness step, training used proneness
//! only, spontaneous used age/condition/jadedness/NF, setback used
//! proneness × medical. The four diverged on which inputs mattered, so
//! a fast jump in workload from training never leaked into match
//! injury risk and vice versa.
//!
//! `compute_injury_risk` takes a `base_rate` (path-specific) and applies
//! a multiplicative envelope of clamped factors:
//!
//!   * **Proneness** — square-root-shaped: 0 → 0.4×, 10 → 1.1×, 20 → 1.8×.
//!   * **Age** — flat under 30, linear ramp over 30 (cap +60% by 36).
//!   * **Natural fitness** — high NF protects, low NF amplifies.
//!   * **Condition** — below 40% adds material risk; above 75% protects.
//!   * **Jadedness** — over 6000 the risk climbs; over 8500 it doubles.
//!   * **Workload spike** — ACWR ≥ 1.4 amplifies up to 1.6× at 2.0+.
//!   * **Match congestion** — every match beyond two in 14d adds risk.
//!   * **Last body part** — recurring injuries: +25%.
//!   * **Recovery phase** — Lmp players carry up to 2.5× the base risk
//!     on heavy intensity (the "first 90 back" recurrence problem).
//!   * **Medical staff** — pass-through multiplier (sports-science
//!     reduces, weak medical raises).
//!
//! Final risk is capped at 8% per call so no roll deterministically
//! injures a player.

use crate::club::player::player::Player;
use crate::utils::DateUtils;
use chrono::NaiveDate;

/// Path-specific inputs. Other modifiers are read off the player.
pub struct InjuryRiskInputs {
    /// Per-path base rate before modifiers (e.g. ~0.0001 for daily
    /// spontaneous, 0.005 × minutes/90 for match).
    pub base_rate: f32,
    /// Activity intensity in "match-equivalent" units. 1.0 = a normal
    /// match minute; 0.4 = video session; 1.6 = a hard pressing drill.
    pub intensity: f32,
    /// True when the player is in the post-injury recovery (Lmp) phase.
    pub in_recovery: bool,
    /// Multiplicative shim from medical staff quality. 1.0 = neutral.
    /// Spontaneous risk pulls this from sports-science, training
    /// pulls it from facility quality, match leaves it 1.0.
    pub medical_multiplier: f32,
    /// Today, used to compute age. Callers usually have this on hand;
    /// passing it in keeps this fn free of clock magic.
    pub now: NaiveDate,
}

impl Player {
    /// Run the unified risk recipe and return a per-tick / per-event
    /// injury chance, capped at 8%. The caller compares the result
    /// against `rand::random::<f32>()` and applies the actual injury.
    pub fn compute_injury_risk(&self, inputs: InjuryRiskInputs) -> f32 {
        let proneness = self.player_attributes.injury_proneness as f32;
        // 0..20 → roughly 0.4..1.8 with a square-root shape (so a 10
        // proneness is ~1.1, not 1.5 like the old linear model).
        let proneness_mult = (0.4 + (proneness / 10.0).sqrt() * 0.7).clamp(0.4, 1.8);

        let age = DateUtils::age(self.birth_date, inputs.now) as f32;
        let age_mult = if age <= 30.0 {
            1.0
        } else {
            (1.0 + (age - 30.0) * 0.10).clamp(1.0, 1.7)
        };

        // Natural fitness (0..20). High NF tendons survive longer.
        let nf = self.skills.physical.natural_fitness;
        let nf_mult = (1.4 - (nf / 20.0) * 0.7).clamp(0.7, 1.4);

        // Condition: below 40% really hurts, above 75% protects slightly.
        let condition_pct = self.player_attributes.condition_percentage() as f32;
        let condition_mult = if condition_pct < 40.0 {
            1.0 + (40.0 - condition_pct) / 50.0
        } else if condition_pct > 75.0 {
            0.92
        } else {
            1.0
        };

        // Jadedness: gentle ramp until 6000, sharper after.
        let jad = self.player_attributes.jadedness as f32;
        let raw_jad_mult = if jad <= 6000.0 {
            1.0 + jad / 30_000.0
        } else if jad <= 8500.0 {
            1.20 + (jad - 6000.0) / 5_000.0
        } else {
            1.70 + (jad - 8500.0) / 5_000.0
        };
        let jad_mult = raw_jad_mult.clamp(1.0, 2.0);

        // Workload spike (ACWR). Only meaningful with a real chronic baseline.
        let spike_ratio = self.load.workload_spike_ratio();
        let raw_spike_mult = if self.load.physical_load_30 < 200.0 {
            1.0
        } else if spike_ratio < 1.0 {
            // Under-loaded — slight risk bump (deconditioning), small.
            1.0 + (1.0 - spike_ratio) * 0.10
        } else if spike_ratio < 1.4 {
            1.0
        } else if spike_ratio < 2.0 {
            1.0 + (spike_ratio - 1.4) * 0.6
        } else {
            1.6
        };
        let spike_mult = raw_spike_mult.clamp(0.95, 1.7);

        // Match congestion: third match in 14 days starts adding risk.
        let matches_14 = self.load.matches_last_14() as f32;
        let congestion_mult = 1.0 + (matches_14 - 2.0).max(0.0) * 0.08;

        // Recurring body part — soft tissue is famously sticky.
        let recurrence_mult = if self.player_attributes.last_injury_body_part != 0 {
            1.25
        } else {
            1.0
        };

        // Recovery phase: Lmp players are at materially higher recurrence
        // risk if they go straight into a heavy session.
        let recovery_mult = if inputs.in_recovery {
            1.0 + 1.5 * inputs.intensity.clamp(0.4, 1.5)
        } else {
            1.0
        };

        // Intensity: 1.0 is a normal match minute. Sub-linear so video
        // sessions don't trivially zero out, super-linear so PressingDrills
        // really do raise risk.
        let intensity_mult = (0.5 + inputs.intensity * 0.6).clamp(0.5, 2.0);

        let medical_mult = inputs.medical_multiplier.clamp(0.4, 1.3);

        let chance = inputs.base_rate
            * proneness_mult
            * age_mult
            * nf_mult
            * condition_mult
            * jad_mult
            * spike_mult
            * congestion_mult
            * recurrence_mult
            * recovery_mult
            * intensity_mult
            * medical_mult;

        chance.clamp(0.0, 0.08)
    }
}
