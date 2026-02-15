use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::state::PlayerState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct DefenderClearingState {}

impl StateProcessingHandler for DefenderClearingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Wait a few ticks before clearing to allow the player to reach the ball
        if ctx.in_state_time < 5 {
            return None;
        }

        let mut state = StateChangeResult::with(PlayerState::Defender(DefenderState::Standing));

        // Get player's position and ball's current position
        let player_position = ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;

        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let field_center_x = field_width / 2.0;
        let field_center_y = field_height / 2.0;

        // Check if ball is at or near a boundary
        const BOUNDARY_THRESHOLD: f32 = 5.0;
        let at_left_boundary = ball_position.x <= BOUNDARY_THRESHOLD;
        let at_right_boundary = ball_position.x >= field_width - BOUNDARY_THRESHOLD;
        let at_top_boundary = ball_position.y >= field_height - BOUNDARY_THRESHOLD;
        let at_bottom_boundary = ball_position.y <= BOUNDARY_THRESHOLD;
        let at_boundary = at_left_boundary || at_right_boundary || at_top_boundary || at_bottom_boundary;

        // Determine the target position for clearing
        let target_position = if at_boundary {
            // If at boundary, clear toward center of field to escape the corner/edge
            // Add variation to avoid predictability
            let offset_x = if at_left_boundary {
                field_width * 0.3
            } else if at_right_boundary {
                field_width * 0.7
            } else {
                field_center_x
            };

            let offset_y = if at_top_boundary {
                field_height * 0.7
            } else if at_bottom_boundary {
                field_height * 0.3
            } else {
                field_center_y
            };

            Vector3::new(offset_x, offset_y, 0.0)
        } else {
            // Normal clear: opposite side of field
            if player_position.x < field_center_x {
                Vector3::new(field_width * 0.8, ball_position.y, 0.0)
            } else {
                Vector3::new(field_width * 0.2, ball_position.y, 0.0)
            }
        };

        // Calculate the direction vector to the target position
        let direction_to_target = (target_position - ball_position).normalize();

        // Use higher clearing speed - especially critical when stuck at boundary
        let clear_speed = if at_boundary { 80.0 } else { 50.0 };

        // Calculate horizontal velocity
        let horizontal_velocity = direction_to_target * clear_speed;

        // Add upward velocity for aerial clearance
        // Higher lift when at boundary to ensure ball escapes
        let z_velocity = if at_boundary { 15.0 } else { 8.0 };

        // Combine horizontal and vertical components
        let ball_velocity = Vector3::new(
            horizontal_velocity.x,
            horizontal_velocity.y,
            z_velocity,
        );

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

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        Some(
            SteeringBehavior::Arrive {
                target: ball_position,
                slowing_distance: 5.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Clearing involves powerful kicking action - explosive effort
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}
