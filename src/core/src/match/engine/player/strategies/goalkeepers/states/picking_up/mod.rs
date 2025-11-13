use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const PICKUP_DISTANCE_THRESHOLD: f32 = 1.0; // Maximum distance to pick up the ball
const PICKUP_SUCCESS_PROBABILITY: f32 = 0.9; // Probability of successfully picking up the ball

#[derive(Default)]
pub struct GoalkeeperPickingUpState {}

impl StateProcessingHandler for GoalkeeperPickingUpState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 0. CRITICAL: Goalkeeper can only pick up balls that are NOT flying away from them
        // If the ball is flying away, they cannot pick it up (e.g., their own pass/kick)
        // Check if ball has significant velocity (not just rolling)
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        if ball_speed > 1.0 && !ctx.ball().is_towards_player_with_angle(0.8) {
            // Ball is flying away from goalkeeper with speed - cannot pick up
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 1. Check if the ball is within pickup distance
        let ball_distance = ctx.ball().distance();
        if ball_distance > PICKUP_DISTANCE_THRESHOLD {
            // Ball is too far to pick up, transition to appropriate state (e.g., Standing)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Attempt to pick up the ball
        let pickup_success = rand::random::<f32>() < PICKUP_SUCCESS_PROBABILITY;
        if pickup_success {
            // Pickup is successful
            let mut state_change =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            // Generate a pickup event
            state_change
                .events
                .add_player_event(PlayerEvent::CaughtBall(ctx.player.id));

            Some(state_change)
        } else {
            // Pickup failed, transition to appropriate state (e.g., Diving)
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the ball to pick it up
        let ball_position = ctx.tick_context.positions.ball.position;
        let direction = (ball_position - ctx.player.position).normalize();
        let speed = ctx.player.skills.physical.pace;
        Some(direction * speed)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Picking up requires moderate intensity with focused effort, includes movement
        GoalkeeperCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
