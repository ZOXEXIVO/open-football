use crate::r#match::defenders::states::DefenderState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct DefenderClearingState {}

impl StateProcessingHandler for DefenderClearingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let mut state = StateChangeResult::with(PlayerState::Defender(DefenderState::Standing));

        // Get player's position and ball's current position
        let player_position = ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;

        // Determine the target position for clearing (opposite side of the field)
        let field_width = ctx.context.field_size.width as f32;

        // If the player is on the left side, clear to the right, and vice versa
        let target_position = if player_position.x < field_width / 2.0 {
            // Clear to the right side of the field
            Vector3::new(field_width, ball_position.y, 0.0)
        } else {
            // Clear to the left side of the field
            Vector3::new(0.0, ball_position.y, 0.0)
        };

        // Calculate the direction vector to the target position
        let direction_to_target = (target_position - ball_position).normalize();

        // Define a speed for clearing the ball, you can adjust this value as needed
        let clear_speed = 30.0;

        // Calculate the velocity vector by scaling the direction vector by the clearing speed
        let ball_velocity = direction_to_target * clear_speed;

        // Add the clear ball event with the calculated velocity
        state
            .events
            .add_player_event(PlayerEvent::ClearBall(ball_velocity));

        // Return the updated state with the clearing event
        Some(state)
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}
