use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};

/// The range at which a forward commits to chasing a loose ball down,
/// as one pair of numbers instead of two states guessing separately.
///
/// # Why this exists
///
/// `ForwardReturningState` entered `Intercepting` whenever the ball was
/// within **200u**; `ForwardInterceptingState` gave the chase up and
/// returned whenever the ball was beyond **150u**. Every ball sitting in
/// the 150-200u band therefore satisfied both conditions at once, and the
/// forward alternated between the two states on consecutive AI ticks for
/// as long as it stayed there — measured at 35,270 `Intercepting` exits
/// in a single match, **100% of them after one tick or less**
/// (`dev_match trace`), and ~18,000 round trips each way.
///
/// Overlapping windows like this cannot settle: the fix is that the
/// give-up distance must lie strictly OUTSIDE the commit distance, with
/// the gap between them acting as the commitment. A forward who has set
/// off after a ball keeps going a little past the point where he would
/// have started — which is what chasing a ball actually looks like.
pub struct InterceptionRange;

impl InterceptionRange {
    /// Ball must be at least this close before a forward sets off after
    /// it. Tightened from the old 200u entry so the commit range and the
    /// give-up range can be ordered without widening the chase.
    pub const COMMIT: f32 = 150.0;
    /// Chase is abandoned once the ball is further away than this.
    /// Strictly greater than `COMMIT` — that ordering is the whole point.
    pub const GIVE_UP: f32 = 180.0;
}

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
