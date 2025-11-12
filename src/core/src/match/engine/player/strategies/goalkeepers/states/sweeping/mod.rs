use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const SWEEPING_DISTANCE_THRESHOLD: f32 = 20.0; // Distance from goal to consider sweeping
const SWEEPING_SPEED_MULTIPLIER: f32 = 1.2; // Multiplier for sweeping speed

#[derive(Default)]
pub struct GoalkeeperSweepingState {}

impl StateProcessingHandler for GoalkeeperSweepingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the ball is within the sweeping distance threshold
        let ball_distance = ctx.ball().distance_to_own_goal();
        if ball_distance > SWEEPING_DISTANCE_THRESHOLD {
            // Ball is too far, transition back to appropriate state (e.g., Standing)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Check if there are any opponents near the ball
        if let Some(_) = ctx.players().opponents().with_ball().next() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the ball to sweep it away
        let ball_position = ctx.tick_context.positions.ball.position;
        let direction = (ball_position - ctx.player.position).normalize();
        let speed = ctx.player.skills.physical.pace * SWEEPING_SPEED_MULTIPLIER;
        Some(direction * speed)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Sweeping requires high intensity as goalkeeper moves far from goal
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
