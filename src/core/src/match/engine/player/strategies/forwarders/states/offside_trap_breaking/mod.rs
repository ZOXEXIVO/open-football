use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardOffsideTrapBreakingState {}

impl StateProcessingHandler for ForwardOffsideTrapBreakingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let mut result = StateChangeResult::new();

        let player_ops = ctx.player();

        // Check if the player is onside
        if player_ops.on_own_side() {
            // Transition to Running state if the player is onside
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if the player has the ball
        if ctx.player.has_ball(ctx) {
            // Transition to Dribbling state if the player has the ball
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // Check if the offside trap is broken
        if !self.is_offside_trap_active(ctx) {
            // Transition to Running state if the offside trap is no longer active
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Find the best position to break the offside trap
        if let Some(target_position) = self.find_best_position(ctx) {
            // Move towards the target position
            let direction = (target_position - ctx.player.position).normalize();
            result.velocity = Some(direction * ctx.player.skills.physical.acceleration);
        }

        Some(result)
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Offside trap breaking is very high intensity - explosive sprint timing
        ForwardCondition::with_velocity(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl ForwardOffsideTrapBreakingState {
    fn is_offside_trap_active(&self, ctx: &StateProcessingContext) -> bool {
        let offside_line = ctx.players().opponents()
            .all()
            .map(|opponent| opponent.position.x)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        // Check if the player is beyond the offside line
        ctx.player.position.x > offside_line
    }

    fn find_best_position(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;

        let offside_line = ctx.players().opponents()
            .all()
            .into_iter()
            .map(|opponent| opponent.position.x)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);

        let target_x = offside_line - 1.0; // Adjust the target position to be just onside
        let target_y = ball_position.y; // Maintain the same y-coordinate as the ball

        Some(Vector3::new(target_x, target_y, 0.0))
    }
}
