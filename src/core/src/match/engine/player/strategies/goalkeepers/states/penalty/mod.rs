use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const PENALTY_SAVE_PROBABILITY: f32 = 0.3; // Probability of saving a penalty

#[derive(Default)]
pub struct GoalkeeperPenaltyState {}

impl StateProcessingHandler for GoalkeeperPenaltyState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the ball is moving towards the goal
        let is_ball_moving_towards_goal = ctx.ball().is_towards_player();

        if !is_ball_moving_towards_goal {
            // Ball is not moving towards the goal, transition to appropriate state (e.g., Standing)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Attempt to save the penalty
        let save_success = rand::random::<f32>() < PENALTY_SAVE_PROBABILITY;
        if save_success {
            // Penalty save is successful
            let mut state_change =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            // Generate a penalty save event
            state_change
                .events
                .add_player_event(PlayerEvent::CaughtBall(ctx.player.id));

            Some(state_change)
        } else {
            // Penalty save failed, transition to appropriate state (e.g., Standing)
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Determine the velocity based on the penalty save attempt
        let save_success = rand::random::<f32>() < PENALTY_SAVE_PROBABILITY;
        if save_success {
            // Move towards the predicted ball position
            let predicted_ball_position = Self::predict_ball_position(ctx);
            let direction = (predicted_ball_position - ctx.player.position).normalize();
            let speed = ctx.player.skills.physical.pace;
            Some(direction * speed)
        } else {
            // Remain stationary
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}

impl GoalkeeperPenaltyState {
    fn predict_ball_position(ctx: &StateProcessingContext) -> Vector3<f32> {
        // Implement ball position prediction logic based on the penalty taker's position and shot direction
        // This can be enhanced with more sophisticated prediction algorithms or machine learning models

        // For simplicity, let's assume the goalkeeper predicts the ball position to be the center of the goal
        ctx.context.goal_positions.left
    }
}
