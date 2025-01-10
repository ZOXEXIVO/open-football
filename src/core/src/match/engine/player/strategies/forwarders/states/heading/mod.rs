use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::result::VectorExtensions;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardHeadingState {}

impl StateProcessingHandler for ForwardHeadingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let mut result = StateChangeResult::new();

        // Check if the ball is within heading range
        if !self.is_ball_within_heading_range(ctx) {
            // Transition to Running state if the ball is not within heading range
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Calculate the heading direction
        let heading_direction = self.calculate_heading_direction(ctx);

        // Perform the heading action
        result.events.add_player_event(PlayerEvent::RequestHeading(
            ctx.player.id,
            heading_direction,
        ));

        // Transition to Running state after heading the ball
        Some(StateChangeResult::with_forward_state(ForwardState::Running))
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardHeadingState {
    fn is_ball_within_heading_range(&self, ctx: &StateProcessingContext) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let heading_range = 1.5; // Adjust based on your game's scale

        ctx.player.position.distance_to(&ball_position) <= heading_range
    }

    fn calculate_heading_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_position = ctx.ball().direction_to_opponent_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        (goal_position - ball_position).normalize()
    }
}
