use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor,
    LOW_CONDITION_THRESHOLD, FIELD_PLAYER_JADEDNESS_INTERVAL, JADEDNESS_INCREMENT,
};

/// Forward-specific activity intensity configuration
pub struct ForwardConfig;

impl ActivityIntensityConfig for ForwardConfig {
    fn very_high_fatigue() -> f32 {
        8.5 // Forwards: highest fatigue for explosive actions
    }

    fn high_fatigue() -> f32 {
        5.5 // Slightly higher than midfielders
    }

    fn moderate_fatigue() -> f32 {
        3.0
    }

    fn low_fatigue() -> f32 {
        1.0
    }

    fn recovery_rate() -> f32 {
        -3.0
    }

    fn sprint_multiplier() -> f32 {
        1.6 // Forwards sprint more often than midfielders
    }

    fn jogging_multiplier() -> f32 {
        0.6
    }

    fn walking_multiplier() -> f32 {
        0.3
    }

    fn low_condition_threshold() -> i16 {
        LOW_CONDITION_THRESHOLD
    }

    fn jadedness_interval() -> u64 {
        FIELD_PLAYER_JADEDNESS_INTERVAL
    }

    fn jadedness_increment() -> i16 {
        JADEDNESS_INCREMENT
    }
}

/// Forward condition processor (type alias for clarity)
pub type ForwardCondition = ConditionProcessor<ForwardConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
