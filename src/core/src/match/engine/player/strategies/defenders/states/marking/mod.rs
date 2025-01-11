use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const MARKING_DISTANCE_THRESHOLD: f32 = 2.0; // Desired distance to maintain from the opponent
const TACKLING_DISTANCE_THRESHOLD: f32 = 1.0; // Distance within which the defender can tackle
const STAMINA_THRESHOLD: f32 = 20.0; // Minimum stamina to continue marking
const BALL_PROXIMITY_THRESHOLD: f32 = 5.0; // Distance to consider the ball as close

#[derive(Default)]
pub struct DefenderMarkingState {}

impl StateProcessingHandler for DefenderMarkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the defender has enough stamina to continue marking
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            // Transition to Resting state if stamina is low
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // 2. Identify the opponent player to mark
        if let Some(opponent_to_mark) = ctx.players().opponents().nearby(100.0).next() {
            let distance_to_opponent = opponent_to_mark.distance(ctx);

            // 4. If the opponent has the ball and is within tackling distance, attempt a tackle
            if opponent_to_mark.has_ball(ctx) && distance_to_opponent < TACKLING_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::SlidingTackle,
                ));
            }

            // 5. If the opponent is beyond the marking distance threshold, switch to Running state to catch up
            if distance_to_opponent > MARKING_DISTANCE_THRESHOLD * 1.5 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Running,
                ));
            }

            // 6. If the ball is close to the defender, consider intercepting
            if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD && !opponent_to_mark.has_ball(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }

            // 7. Continue marking (no state change)
            None
        } else {
            // No opponent to mark found
            // Transition back to HoldingLine or appropriate state
            Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        // For now, return None to indicate no state change
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move to maintain position relative to the opponent being marked

        // Identify the opponent player to mark
        if let Some(opponent_to_mark) = ctx.players().opponents().nearby(100.0).next() {
            // Calculate desired position to maintain proper marking
            let opponent_future_position = opponent_to_mark.position + opponent_to_mark.velocity(ctx);
            let desired_position = opponent_future_position
                - (opponent_to_mark.velocity(ctx).normalize() * MARKING_DISTANCE_THRESHOLD);

            let direction = (desired_position - ctx.player.position).normalize();
            // Set speed based on player's pace
            let speed = ctx.player.skills.physical.pace; // Use pace attribute
            Some(direction * speed)
        } else {
            // No opponent to mark found
            // Remain stationary or return to default position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}

impl DefenderMarkingState {

}
