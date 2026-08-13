use crate::r#match::StateProcessingContext;
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};
use crate::r#match::midfielders::states::MidfielderState;

/// Where a midfielder's shape position is, and whether he is far enough
/// off it to be worth a recovery run.
pub struct ShapeStation;

impl ShapeStation {
    /// Recovery run is complete inside this radius. Same value
    /// `MidfielderReturningState` has always used to hand back to
    /// `Running`, so "arrived" means exactly what it did before.
    pub const HOME: f32 = 80.0;
    /// A midfielder already home is only sent back out once he has
    /// drifted past this. The gap to `HOME` is the commitment that stops
    /// the two states contradicting each other.
    pub const DISPLACED: f32 = 110.0;

    /// Distance from the player's shape position.
    pub fn drift(ctx: &StateProcessingContext) -> f32 {
        (ctx.player.position - ctx.player.start_position).magnitude()
    }

    /// Is there a recovery run left to make?
    ///
    /// Hysteretic on the current state: a player already in `Returning`
    /// keeps going until he is inside `HOME`, while one who is not has to
    /// have drifted past `DISPLACED` before a fresh recovery is worth
    /// starting. Both sides of the hand-off read this, so neither can
    /// contradict the other.
    pub fn should_recover(ctx: &StateProcessingContext) -> bool {
        let returning = matches!(
            ctx.player.state,
            crate::r#match::player::state::PlayerState::Midfielder(MidfielderState::Returning)
        );
        let band = if returning {
            Self::HOME
        } else {
            Self::DISPLACED
        };
        Self::drift(ctx) > band
    }
}

/// Midfielder-specific activity intensity configuration
pub struct MidfielderConfig;

impl ActivityIntensityConfig for MidfielderConfig {
    fn very_high_fatigue() -> f32 {
        8.0 // Explosive actions tire quickly
    }

    fn high_fatigue() -> f32 {
        5.0 // Base from running state
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
        1.5 // Sprinting
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

/// Midfielder condition processor (type alias for clarity)
pub type MidfielderCondition = ConditionProcessor<MidfielderConfig>;

// Re-export for convenience
pub use crate::r#match::engine::player::strategies::common::ActivityIntensity;
