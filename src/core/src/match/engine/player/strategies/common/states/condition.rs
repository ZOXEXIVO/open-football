use super::activity_intensity::{ActivityIntensity, ActivityIntensityConfig};
use super::constants::{
    FATIGUE_RATE_MULTIPLIER, MATCH_CONDITION_FLOOR, MAX_CONDITION, MAX_JADEDNESS,
    RECOVERY_RATE_MULTIPLIER,
};
use crate::r#match::ConditionContext;
use log::trace;
use std::marker::PhantomData;

/// Generic condition processor with role-specific configurations
pub struct ConditionProcessor<T: ActivityIntensityConfig> {
    intensity: ActivityIntensity,
    _phantom: PhantomData<T>,
}

impl<T: ActivityIntensityConfig> ConditionProcessor<T> {
    /// Create a new condition processor (always uses velocity-based calculation)
    pub fn new(intensity: ActivityIntensity) -> Self {
        Self {
            intensity,
            _phantom: PhantomData,
        }
    }

    /// Create a new condition processor with velocity-based intensity (deprecated, use new())
    /// Kept for backward compatibility
    pub fn with_velocity(intensity: ActivityIntensity) -> Self {
        Self::new(intensity)
    }

    /// Process condition changes based on activity intensity and player attributes
    /// Calculation: 75% velocity-based, 25% intensity-based
    pub fn process(self, ctx: ConditionContext) {
        // Record the exertion this state declared so the movement
        // integrator can scale speed to match. One signal feeds both
        // fatigue and movement, so the two can never drift apart.
        ctx.player.last_activity_intensity = self.intensity;

        let stamina_skill = ctx.player.skills.physical.stamina;
        let natural_fitness = ctx.player.skills.physical.natural_fitness;
        let chronic_fitness = ctx.player.player_attributes.fitness;
        let jadedness = ctx.player.player_attributes.jadedness;
        let recovery_debt = ctx.player.starting_recovery_debt;

        // Stamina affects how tired the player gets (better stamina = less fatigue)
        // Range: 0.5x to 1.5x (high stamina players tire 50% slower)
        let stamina_factor = 1.5 - (stamina_skill / 20.0);

        // Natural fitness affects recovery and fatigue resistance
        let fitness_factor = 1.3 - (natural_fitness / 20.0) * 0.6;

        // Chronic fitness (training base) further tilts tick fatigue:
        // a player in deep season-long form tires slower than a
        // recently-returned-from-injury one with the same NF/stamina.
        // 0.90..1.15× band: peak fitness ≈ 0.90, baseline ≈ 1.15.
        let chronic_fitness01 = (chronic_fitness as f32 / 10_000.0).clamp(0.0, 1.0);
        let chronic_mult = 1.15 - chronic_fitness01 * 0.25;

        // Recovery-debt and jadedness pile-on. A player walking into a
        // match with deep debt or chronic tiredness drains faster on
        // top of everything else — back-to-back fixtures finally cost
        // the legs the engine knew about, not just the post-match
        // bookkeeping.
        let debt01 = (recovery_debt / 1_500.0).clamp(0.0, 1.0);
        let debt_mult = 1.0 + debt01 * 0.35;
        let jaded_mult = 1.0 + (jadedness as f32 / 10_000.0).clamp(0.0, 1.0) * 0.20;

        // Calculate velocity-based fatigue (75% of total effect)
        // Use squared values to avoid sqrt — compare ratio² against threshold²
        let velocity_sq = ctx.player.velocity.norm_squared();
        let max_speed = ctx.player.max_speed_with_condition_cached();
        let max_speed_sq = max_speed * max_speed;

        // intensity_ratio_sq = (speed / max_speed)²
        let intensity_ratio_sq = if max_speed_sq > 0.0 {
            (velocity_sq / max_speed_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };

        #[cfg(feature = "match-logs")]
        {
            use crate::r#match::player::strategies::players::ops::forward_shot_decision::time_band_diag;
            use std::sync::atomic::Ordering;
            let band = if intensity_ratio_sq < 0.0025 {
                0
            } else if intensity_ratio_sq < 0.09 {
                1
            } else if intensity_ratio_sq < 0.36 {
                2
            } else if intensity_ratio_sq < 0.7225 {
                3
            } else {
                4
            };
            // Outfield only — GK condition is near-constant and would
            // drown the signal this counter exists to expose.
            if T::sprint_multiplier() > 1.4 {
                time_band_diag::VELOCITY_BAND_TICKS[band].fetch_add(1, Ordering::Relaxed);
            }
        }

        // Continuous metabolic curve (replaces the old 5-band step
        // function: -6 / -2 / +3 / +6 / +9). Below the break-even pace the
        // legs recover; above it the drain climbs smoothly toward a
        // role-dependent sprint peak, accelerating as the player nears
        // flat-out — matching how real energy cost rises steeply with
        // pace. Continuous matters now that effort + self-pacing
        // (`MovementEffort`) put players at intermediate speeds: a step
        // function would reintroduce a drain cliff at the band edges (a
        // 2%-of-max-speed change swinging fatigue by 50%). The anchor
        // points still track the old bands — jog≈+2.3, run≈+5.8,
        // sprint≈peak — so the calibrated trajectory is preserved.
        let intensity_ratio = intensity_ratio_sq.sqrt();
        const BREAK_EVEN: f32 = 0.30;
        let sprint_peak = if T::sprint_multiplier() > 1.55 {
            10.0 // Forwards (highest)
        } else if T::sprint_multiplier() > 1.4 {
            9.0 // Defenders / Midfielders
        } else {
            7.0 // Goalkeepers (lowest)
        };
        let velocity_fatigue = if intensity_ratio < BREAK_EVEN {
            // Recovery: deepest when fully stationary (−6.0 at rest),
            // easing to zero at the break-even pace.
            -6.0 * (1.0 - intensity_ratio / BREAK_EVEN)
        } else {
            let over = (intensity_ratio - BREAK_EVEN) / (1.0 - BREAK_EVEN);
            sprint_peak * over.powf(1.15)
        };

        // Calculate intensity-based fatigue modifier (25% of total effect)
        let base_intensity_fatigue = self.intensity.base_fatigue::<T>();

        // Normalize intensity contribution to be smaller
        let intensity_fatigue = base_intensity_fatigue * 0.3;

        // Combine: 75% velocity + 25% intensity
        let combined_fatigue = velocity_fatigue * 0.75 + intensity_fatigue * 0.25;

        // Match-progress fatigue curve. Real football: every minute
        // costs more than the last — even the first half leaves legs
        // feeling heavier by the time the whistle blows. Previously
        // the curve only ramped after half-time, which meant the first
        // 45 minutes were effectively fatigue-free. New curve:
        //   minute  0 :  1.15× fatigue  /  0.95× recovery
        //   minute 45 :  1.33× fatigue  /  0.80× recovery
        //   minute 90 :  1.50× fatigue  /  0.65× recovery
        // Linear ramp from kickoff gives every phase of the match a
        // stamina cost — early pressing sides now actually tire out,
        // and late substitutes enter a genuinely fatigued field.
        let late_match_fatigue_mult = 1.15 + ctx.match_progress * 0.35;
        let late_match_recovery_mult = 0.95 - ctx.match_progress * 0.30;

        // Apply rate multiplier based on whether it's fatigue or recovery
        let rate_multiplier = if combined_fatigue < 0.0 {
            RECOVERY_RATE_MULTIPLIER * late_match_recovery_mult
        } else {
            FATIGUE_RATE_MULTIPLIER * late_match_fatigue_mult
        };

        // Chronic / debt / jadedness multipliers only inflate FATIGUE,
        // not recovery — a player with heavy legs still recovers at a
        // normal rate when standing still (the body just doesn't get
        // any extra credit). Applying them to negative changes would
        // also cancel each other out in odd ways.
        let load_mult = if combined_fatigue > 0.0 {
            chronic_mult * debt_mult * jaded_mult
        } else {
            1.0
        };

        let condition_change_f =
            combined_fatigue * stamina_factor * fitness_factor * rate_multiplier * load_mult;

        // Accumulate fractional fatigue to avoid float-to-int truncation losing small per-tick values
        ctx.player.fatigue_accumulator += condition_change_f;

        // Only apply when accumulator reaches a full integer point
        let condition_change = ctx.player.fatigue_accumulator as i16;
        if condition_change != 0 {
            ctx.player.fatigue_accumulator -= condition_change as f32;

            let old_condition = ctx.player.player_attributes.condition;

            // Apply condition change (clamped to MATCH_CONDITION_FLOOR..MAX_CONDITION)
            // In FM, condition never drops below ~30% even during the most intense match
            ctx.player.player_attributes.condition = (ctx.player.player_attributes.condition
                - condition_change)
                .clamp(MATCH_CONDITION_FLOOR, MAX_CONDITION);

            trace!(
                "Condition: player={}, vel_sq={:.3}, change={}, acc={:.3}, condition: {} -> {}",
                ctx.player.id,
                velocity_sq,
                condition_change,
                ctx.player.fatigue_accumulator,
                old_condition,
                ctx.player.player_attributes.condition
            );
        }

        // If condition drops very low, slightly increase jadedness (long-term tiredness)
        if ctx.player.player_attributes.condition < T::low_condition_threshold()
            && ctx.in_state_time % T::jadedness_interval() == 0
        {
            // Increase jadedness slightly when very tired
            ctx.player.player_attributes.jadedness = (ctx.player.player_attributes.jadedness
                + T::jadedness_increment())
            .min(MAX_JADEDNESS);
        }
    }
}
