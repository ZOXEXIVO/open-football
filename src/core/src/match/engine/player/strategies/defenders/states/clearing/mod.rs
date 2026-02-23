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

        // Get ball's current position
        let ball_position = ctx.tick_context.positions.ball.position;

        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let field_center_y = field_height / 2.0;

        // Check if ball is at or near a boundary
        const BOUNDARY_THRESHOLD: f32 = 5.0;
        let at_left_boundary = ball_position.x <= BOUNDARY_THRESHOLD;
        let at_right_boundary = ball_position.x >= field_width - BOUNDARY_THRESHOLD;
        let at_top_boundary = ball_position.y >= field_height - BOUNDARY_THRESHOLD;
        let at_bottom_boundary = ball_position.y <= BOUNDARY_THRESHOLD;
        let at_boundary = at_left_boundary || at_right_boundary || at_top_boundary || at_bottom_boundary;

        // Determine clearance direction based on player's side (always clear AWAY from own goal)
        let is_left_side = ctx.player.side == Some(crate::r#match::player::PlayerSide::Left);

        // Clearance distance upfield
        const CLEAR_DISTANCE: f32 = 25.0;

        // Target X: upfield away from own goal
        let target_x = if is_left_side {
            (ball_position.x + CLEAR_DISTANCE).min(field_width * 0.55)
        } else {
            (ball_position.x - CLEAR_DISTANCE).max(field_width * 0.45)
        };

        // Target Y: always pull toward field center to stay infield
        // Blend ball's current Y toward center — the further from center, the stronger the pull
        let center_pull = 0.6; // 60% toward center
        let target_y = ball_position.y + (field_center_y - ball_position.y) * center_pull;

        let target_position = Vector3::new(target_x, target_y, 0.0);

        // Calculate the direction vector to the target position
        let direction_to_target = (target_position - ball_position).normalize();

        // Horizontal speed: moderate so ball doesn't fly across the entire pitch
        let clear_speed = if at_boundary { 5.0 } else { 4.0 };

        // Calculate horizontal velocity
        let horizontal_velocity = direction_to_target * clear_speed;

        // High z velocity for a proper lofted clearance (boot it into the air)
        let z_velocity = if at_boundary { 6.0 } else { 5.0 };

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
