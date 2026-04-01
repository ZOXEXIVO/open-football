use crate::r#match::ConditionContext;
use log::trace;
use super::activity_intensity::{ActivityIntensity, ActivityIntensityConfig};
use super::constants::{MAX_CONDITION, MATCH_CONDITION_FLOOR, MAX_JADEDNESS, FATIGUE_RATE_MULTIPLIER, RECOVERY_RATE_MULTIPLIER};

/// Generic condition processor with role-specific configurations
pub struct ConditionProcessor<T: ActivityIntensityConfig> {
    intensity: ActivityIntensity,
    _phantom: std::marker::PhantomData<T>,
}

impl<T: ActivityIntensityConfig> ConditionProcessor<T> {
    /// Create a new condition processor (always uses velocity-based calculation)
    pub fn new(intensity: ActivityIntensity) -> Self {
        Self {
            intensity,
            _phantom: std::marker::PhantomData,
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
        let stamina_skill = ctx.player.skills.physical.stamina;
        let natural_fitness = ctx.player.skills.physical.natural_fitness;

        // Stamina affects how tired the player gets (better stamina = less fatigue)
        // Range: 0.5x to 1.5x (high stamina players tire 50% slower)
        let stamina_factor = 1.5 - (stamina_skill / 20.0);

        // Natural fitness affects recovery and fatigue resistance
        let fitness_factor = 1.3 - (natural_fitness / 20.0) * 0.6;

        // Calculate velocity-based fatigue (75% of total effect)
        // Use squared values to avoid sqrt — compare ratio² against threshold²
        let velocity_sq = ctx.player.velocity.norm_squared();
        let max_speed = ctx.player.skills.max_speed_with_condition(
            ctx.player.player_attributes.condition,
        );
        let max_speed_sq = max_speed * max_speed;

        // intensity_ratio_sq = (speed / max_speed)²
        let intensity_ratio_sq = if max_speed_sq > 0.0 {
            (velocity_sq / max_speed_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Compare against squared thresholds: 0.05² = 0.0025, 0.3² = 0.09, 0.6² = 0.36, 0.85² = 0.7225
        let velocity_fatigue = if intensity_ratio_sq < 0.0025 {
            -4.0 * 1.5 // Nearly stationary - recovery
        } else if intensity_ratio_sq < 0.09 {
            -2.0 // Walking slowly - light recovery
        } else if intensity_ratio_sq < 0.36 {
            3.0 // Jogging
        } else if intensity_ratio_sq < 0.7225 {
            6.0 // Running
        } else {
            // Sprinting - varies by role
            if T::sprint_multiplier() > 1.55 {
                10.0 // Forwards (highest)
            } else if T::sprint_multiplier() > 1.4 {
                9.0 // Defenders/Midfielders
            } else {
                7.0 // Goalkeepers (lowest)
            }
        };

        // Calculate intensity-based fatigue modifier (25% of total effect)
        let base_intensity_fatigue = self.intensity.base_fatigue::<T>();

        // Normalize intensity contribution to be smaller
        let intensity_fatigue = base_intensity_fatigue * 0.3;

        // Combine: 75% velocity + 25% intensity
        let combined_fatigue = velocity_fatigue * 0.75 + intensity_fatigue * 0.25;

        // Apply rate multiplier based on whether it's fatigue or recovery
        let rate_multiplier = if combined_fatigue < 0.0 {
            RECOVERY_RATE_MULTIPLIER
        } else {
            FATIGUE_RATE_MULTIPLIER
        };

        let condition_change_f = combined_fatigue * stamina_factor * fitness_factor * rate_multiplier;

        // Accumulate fractional fatigue to avoid float-to-int truncation losing small per-tick values
        ctx.player.fatigue_accumulator += condition_change_f;

        // Only apply when accumulator reaches a full integer point
        let condition_change = ctx.player.fatigue_accumulator as i16;
        if condition_change != 0 {
            ctx.player.fatigue_accumulator -= condition_change as f32;

            let old_condition = ctx.player.player_attributes.condition;

            // Apply condition change (clamped to MATCH_CONDITION_FLOOR..MAX_CONDITION)
            // In FM, condition never drops below ~30% even during the most intense match
            ctx.player.player_attributes.condition =
                (ctx.player.player_attributes.condition - condition_change).clamp(MATCH_CONDITION_FLOOR, MAX_CONDITION);

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
            && ctx.in_state_time % T::jadedness_interval() == 0 {
            // Increase jadedness slightly when very tired
            ctx.player.player_attributes.jadedness =
                (ctx.player.player_attributes.jadedness + T::jadedness_increment()).min(MAX_JADEDNESS);
        }
    }
}
