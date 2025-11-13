use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor,
    GOALKEEPER_LOW_CONDITION_THRESHOLD, GOALKEEPER_JADEDNESS_INTERVAL,
    GOALKEEPER_JADEDNESS_INCREMENT,
};

/// Goalkeeper-specific activity intensity configuration
pub struct GoalkeeperConfig;

impl ActivityIntensityConfig for GoalkeeperConfig {
    fn very_high_fatigue() -> f32 {
        7.0 // Lower than outfield players - explosive but infrequent
    }

    fn high_fatigue() -> f32 {
        4.5 // Lower than outfield players
    }

    fn moderate_fatigue() -> f32 {
        2.5
    }

    fn low_fatigue() -> f32 {
        0.8
    }

    fn recovery_rate() -> f32 {
        -4.0 // Better recovery than outfield players
    }

    fn sprint_multiplier() -> f32 {
        1.3 // Sprinting (less demanding than outfield players)
    }

    fn jogging_multiplier() -> f32 {
        0.5
    }

    fn walking_multiplier() -> f32 {
        0.2
    }

    fn low_condition_threshold() -> i16 {
        GOALKEEPER_LOW_CONDITION_THRESHOLD
    }

    fn jadedness_interval() -> u64 {
        GOALKEEPER_JADEDNESS_INTERVAL
    }

    fn jadedness_increment() -> i16 {
        GOALKEEPER_JADEDNESS_INCREMENT
    }
}

/// Goalkeeper condition processor (type alias for clarity)
pub type GoalkeeperCondition = ConditionProcessor<GoalkeeperConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
