use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const BLOCK_DISTANCE_THRESHOLD: f32 = 5.0; // Increased from 2.0 - wider blocking zone
const MAX_BLOCK_TIME: u64 = 500; // Don't stay in blocking state too long
const SHOT_SPEED_THRESHOLD: f32 = 6.0; // Ball speed indicating a shot

#[derive(Default)]
pub struct DefenderBlockingState {}

impl StateProcessingHandler for DefenderBlockingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing
            ));
        }

        // Exit blocking if too much time has passed
        if ctx.in_state_time > MAX_BLOCK_TIME {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // If ball has slowed down or stopped, exit blocking
        if ball_speed < SHOT_SPEED_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // If ball is too far, exit blocking
        if ctx.ball().distance() > BLOCK_DISTANCE_THRESHOLD * 3.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // If ball is not coming toward us anymore, exit
        if !ctx.ball().is_towards_player_with_angle(0.5) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // If ball is moving fast, try to intercept its path
        if ball_speed > SHOT_SPEED_THRESHOLD {
            // Predict where the ball will be
            let prediction_time = 0.3; // Look ahead 300ms
            let predicted_ball_pos = ball_position + ball_velocity * prediction_time;

            // Move toward predicted interception point
            let to_intercept = predicted_ball_pos - ctx.player.position;
            let intercept_distance = to_intercept.magnitude();

            if intercept_distance > 1.0 {
                // Use quick lateral movement to get in front of the ball
                let direction = to_intercept.normalize();
                let speed = ctx.player.skills.physical.pace * 0.8; // Quick but controlled
                return Some(direction * speed);
            }
        }

        // Position between ball and own goal
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_to_goal = (own_goal - ball_position).normalize();
        let blocking_position = ball_position + ball_to_goal * 2.0; // 2 meters in front of ball toward goal

        Some(
            SteeringBehavior::Arrive {
                target: blocking_position,
                slowing_distance: 3.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Blocking requires quick reactions - moderate intensity
        DefenderCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}
