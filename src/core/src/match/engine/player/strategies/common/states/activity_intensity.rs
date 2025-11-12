/// Activity intensity levels for condition processing
#[derive(Debug, Clone, Copy)]
pub enum ActivityIntensity {
    /// Very high intensity - explosive actions (shooting, finishing, heading, tackling, sliding)
    VeryHigh,
    /// High intensity - sustained running, pressing, intercepting, dribbling
    High,
    /// Moderate intensity - assisting, creating space, marking, tracking, returning, covering
    Moderate,
    /// Low intensity - walking, passing
    Low,
    /// Recovery - standing still, resting, holding line with minimal movement
    Recovery,
}

/// Trait for role-specific activity intensity configurations
pub trait ActivityIntensityConfig {
    /// Get the base fatigue for very high intensity activities
    fn very_high_fatigue() -> f32;

    /// Get the base fatigue for high intensity activities
    fn high_fatigue() -> f32;

    /// Get the base fatigue for moderate intensity activities
    fn moderate_fatigue() -> f32;

    /// Get the base fatigue for low intensity activities
    fn low_fatigue() -> f32;

    /// Get the recovery rate (negative value)
    fn recovery_rate() -> f32;

    /// Get the sprint intensity multiplier
    fn sprint_multiplier() -> f32;

    /// Get the running intensity multiplier
    fn running_multiplier() -> f32 {
        1.0 // Default for all roles
    }

    /// Get the jogging intensity multiplier
    fn jogging_multiplier() -> f32;

    /// Get the walking intensity multiplier
    fn walking_multiplier() -> f32;

    /// Get the low condition threshold for jadedness
    fn low_condition_threshold() -> i16;

    /// Get the jadedness check interval
    fn jadedness_interval() -> u64;

    /// Get the jadedness increment per check
    fn jadedness_increment() -> i16;
}

impl ActivityIntensity {
    /// Get the base fatigue for this activity intensity with the given config
    pub fn base_fatigue<T: ActivityIntensityConfig>(&self) -> f32 {
        match self {
            ActivityIntensity::VeryHigh => T::very_high_fatigue(),
            ActivityIntensity::High => T::high_fatigue(),
            ActivityIntensity::Moderate => T::moderate_fatigue(),
            ActivityIntensity::Low => T::low_fatigue(),
            ActivityIntensity::Recovery => T::recovery_rate(),
        }
    }
}
