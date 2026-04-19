use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

// Real-world penalty save rate is ~20-25%; we derive it from keeper skills.
const PENALTY_SAVE_BASE: f32 = 0.10;

fn penalty_save_probability(ctx: &StateProcessingContext) -> f32 {
    let reflexes = (ctx.player.skills.goalkeeping.reflexes - 1.0) / 19.0;
    let one_on_ones = (ctx.player.skills.goalkeeping.one_on_ones - 1.0) / 19.0;
    let anticipation = (ctx.player.skills.mental.anticipation - 1.0) / 19.0;
    let skill = reflexes * 0.5 + one_on_ones * 0.3 + anticipation * 0.2;
    (PENALTY_SAVE_BASE + skill * 0.22).clamp(0.08, 0.35)
}

#[derive(Default, Clone)]
pub struct GoalkeeperPenaltyState {}

impl StateProcessingHandler for GoalkeeperPenaltyState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the ball is moving towards the goal
        let is_ball_moving_towards_goal = ctx.ball().is_towards_player();

        if !is_ball_moving_towards_goal {
            // Ball is not moving towards the goal, transition to appropriate state (e.g., Standing)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Attempt to save the penalty
        let save_success = rand::random::<f32>() < penalty_save_probability(ctx);
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


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Determine the velocity based on the penalty save attempt
        let save_success = rand::random::<f32>() < penalty_save_probability(ctx);
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

    fn process_conditions(&self, ctx: ConditionContext) {
        // Penalty saves require very high intensity with explosive effort
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
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
