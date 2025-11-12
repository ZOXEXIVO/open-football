use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const PUNCHING_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt punching
const PUNCH_SUCCESS_PROBABILITY: f32 = 0.8; // Probability of a successful punch

#[derive(Default)]
pub struct GoalkeeperPunchingState {}

impl StateProcessingHandler for GoalkeeperPunchingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.ball().distance() > PUNCHING_DISTANCE_THRESHOLD {
            // Ball is too far to punch, transition to appropriate state (e.g., Jumping)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Jumping,
            ));
        }

        // 2. Attempt to punch the ball
        let punch_success = rand::random::<f32>() < PUNCH_SUCCESS_PROBABILITY;
        if punch_success {
            // Punch is successful
            let mut state_change =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::Standing);

            // Determine the direction to punch the ball (e.g., towards the sidelines)
            let punch_direction = ctx.ball().direction_to_own_goal().normalize() * -1.0;

            // Generate a punch event
            state_change
                .events
                .add_player_event(PlayerEvent::ClearBall(punch_direction));

            Some(state_change)
        } else {
            // Punch failed, transition to appropriate state (e.g., Diving)
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Remain stationary while punching
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Punching is a very high intensity activity requiring explosive effort
        GoalkeeperCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
