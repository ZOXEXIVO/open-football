use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MARKING_DISTANCE: f32 = 15.0;
const INTERCEPTION_DISTANCE: f32 = 100.0;
const FIELD_THIRD_THRESHOLD: f32 = 0.33;
const PUSH_UP_HYSTERESIS: f32 = 0.05;

#[derive(Default)]
pub struct DefenderCoveringState {}

impl StateProcessingHandler for DefenderCoveringState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.in_state_time < 300 {
            return None;
        }

        let ball_ops = ctx.ball();
        if ball_ops.on_own_side() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        if ball_ops.distance_to_opponent_goal()
            < ctx.context.field_size.width as f32 * (FIELD_THIRD_THRESHOLD - PUSH_UP_HYSTERESIS)
            && self.should_push_up(ctx)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::PushingUp,
            ));
        }

        if let Some(_) = ctx.players().opponents().nearby(MARKING_DISTANCE).next() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Marking,
            ));
        }

        if ball_ops.is_towards_player() && ball_ops.distance() < INTERCEPTION_DISTANCE {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: self.calculate_optimal_covering_position(ctx)
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl DefenderCoveringState {
    fn should_push_up(&self, ctx: &StateProcessingContext) -> bool {
        let ball_ops = ctx.ball();
        let player_ops = ctx.player();

        let ball_in_attacking_third = ball_ops.distance_to_opponent_goal()
            < ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD;
        let team_in_possession = ctx.team().is_control_ball();
        let defender_not_last_man = !self.is_last_defender(ctx);

        ball_in_attacking_third
            && team_in_possession
            && defender_not_last_man
            && player_ops.distance_from_start_position()
                < ctx.context.field_size.width as f32 * 0.25
    }

    fn is_last_defender(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().teammates().defenders()
            .all(|d| d.position.x >= ctx.player.position.x)
    }

    fn calculate_optimal_covering_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Calculate the center of the middle third with slight offset towards own goal
        let middle_third_center = Vector3::new(
            field_width * 0.4, // Moved slightly back from 0.5
            field_height * 0.5,
            0.0,
        );

        // Get direction to own goal and normalize it
        let ball_to_goal = (ctx.ball().direction_to_own_goal() - ball_position).normalize();

        // Calculate base covering position with better distance scaling
        let covering_distance = (ball_position - ctx.ball().direction_to_own_goal()).magnitude() * 0.35;
        let covering_position = ball_position + ball_to_goal * covering_distance.min(field_width * 0.3);

        // Apply exponential moving average for position smoothing
        const SMOOTHING_FACTOR: f32 = 0.15; // Adjust this value (0.0 to 1.0) to control smoothing
        let previous_position = ctx.player.position;

        // Calculate blended position with weighted factors
        let target_position = Vector3::new(
            covering_position.x * 0.5 +  // Reduced weight from 0.6
                middle_third_center.x * 0.4 + // Increased weight from 0.3
                player_position.x * 0.1,
            covering_position.y * 0.5 +
                middle_third_center.y * 0.4 +
                player_position.y * 0.1,
            0.0,
        );

        // Apply smoothing between frames
        let smoothed_position = previous_position.lerp(&target_position, SMOOTHING_FACTOR);

        // Ensure the position stays within reasonable bounds
        let max_distance_from_center = field_width * 0.35;
        let position_relative_to_center = smoothed_position - middle_third_center;
        let capped_position = if position_relative_to_center.magnitude() > max_distance_from_center {
            middle_third_center + position_relative_to_center.normalize() * max_distance_from_center
        } else {
            smoothed_position
        };

        // Final boundary check
        Vector3::new(
            capped_position.x.clamp(field_width * 0.1, field_width * 0.7),  // Prevent getting too close to either goal
            capped_position.y.clamp(field_height * 0.1, field_height * 0.9), // Keep away from sidelines
            0.0,
        )
    }
}
