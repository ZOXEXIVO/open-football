pub mod read;

use crate::r#match::engine::ball::ball::OffsideLine;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::strategies::common::players::ops::forward_shot_decision::{
    ShotDecision, evaluate_forward_shot_decision,
};
use crate::r#match::{StateChangeResult, StateProcessingContext};
use crate::r#match::engine::player::strategies::common::{
    ActivityIntensityConfig, ConditionProcessor, FIELD_PLAYER_JADEDNESS_INTERVAL,
    JADEDNESS_INCREMENT, LOW_CONDITION_THRESHOLD,
};

pub use read::{ForwardRead, ReadLevel};

/// The line a forward's run is judged against, read as the referee reads
/// it: `OffsideLine::for_run` over every opponent.
pub struct RunLine;

impl RunLine {
    pub fn of(ctx: &StateProcessingContext) -> Option<f32> {
        OffsideLine::for_run(
            ctx.players().opponents().all().map(|o| o.position.x),
            ctx.player.side?,
            ctx.context.field_size.width as f32 / 2.0,
        )
    }

    /// Is he in an offside position now: beyond the line and the ball?
    pub fn is_offside(ctx: &StateProcessingContext) -> bool {
        let (Some(side), Some(line)) = (ctx.player.side, Self::of(ctx)) else {
            return false;
        };
        OffsideLine::is_beyond(
            side,
            ctx.player.position.x,
            ctx.tick_context.positions.ball.position.x,
            line,
        )
    }
}

/// How a forward's state asks for a shot.
pub struct ForwardShot;

impl ForwardShot {
    /// Six metres from goal (48 u). Inside it a strike is a finish — a
    /// first-time contact where hesitating costs the chance — not a shot
    /// from range, and a forward on the ball shoots rather than dribbling
    /// at the keeper.
    pub const POINT_BLANK: f32 = 48.0;

    /// A candidate shot routed through the shot helper: `Shooting` when it
    /// approves the strike, `Passing` when a team-mate has the better look,
    /// and `None` to let the caller fall through to its next priority. A
    /// direct `with_shot_reason` would skip the helper's gates, because
    /// `ForwardShootingState` does not roll again once a reason is set.
    pub fn dispatch(ctx: &StateProcessingContext, tag: &'static str) -> Option<StateChangeResult> {
        match evaluate_forward_shot_decision(ctx, tag) {
            ShotDecision::Shoot { reason } => Some(
                StateChangeResult::with_forward_state(ForwardState::Shooting)
                    .with_shot_reason(reason),
            ),
            ShotDecision::Pass => Some(StateChangeResult::with_forward_state(ForwardState::Passing)),
            ShotDecision::Hold => None,
        }
    }
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
