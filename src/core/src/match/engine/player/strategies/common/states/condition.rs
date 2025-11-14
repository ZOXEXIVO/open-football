use crate::r#match::ConditionContext;
use super::activity_intensity::{ActivityIntensity, ActivityIntensityConfig};
use super::constants::{MAX_CONDITION, MAX_JADEDNESS, FATIGUE_RATE_MULTIPLIER, RECOVERY_RATE_MULTIPLIER};

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
        let velocity_magnitude = ctx.player.velocity.norm();
        let max_speed = ctx.player.skills.max_speed_with_condition(
            ctx.player.player_attributes.condition,
        );

        let velocity_fatigue = if velocity_magnitude < 0.3 {
            // Resting - recovery
            -4.0 * 1.5 // Negative = recovery, boosted for visibility
        } else {
            let intensity_ratio = if max_speed > 0.0 {
                (velocity_magnitude / max_speed).clamp(0.0, 1.0)
            } else {
                0.5
            };

            // Velocity-based fatigue: scales from 0 (walking) to 10 (sprinting)
            if intensity_ratio < 0.3 {
                1.0 // Walking slowly
            } else if intensity_ratio < 0.6 {
                3.0 // Jogging
            } else if intensity_ratio < 0.85 {
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

        let condition_change = (combined_fatigue * stamina_factor * fitness_factor * rate_multiplier) as i16;

        // Apply condition change (clamped to 0..MAX_CONDITION)
        ctx.player.player_attributes.condition =
            (ctx.player.player_attributes.condition - condition_change).clamp(0, MAX_CONDITION);

        // If condition drops very low, slightly increase jadedness (long-term tiredness)
        if ctx.player.player_attributes.condition < T::low_condition_threshold()
            && ctx.in_state_time % T::jadedness_interval() == 0 {
            // Increase jadedness slightly when very tired
            ctx.player.player_attributes.jadedness =
                (ctx.player.player_attributes.jadedness + T::jadedness_increment()).min(MAX_JADEDNESS);
        }
    }
}
