use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardInterceptingState {}

impl StateProcessingHandler for ForwardInterceptingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        let ball_distance = ctx.ball().distance();

        if ball_distance > 150.0 {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        if ball_distance < 30.0 && ctx.tick_context.ball.is_owned {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Tackling,
            ));
        }

        // 2. Check if the defender can reach the interception point before any opponent
        if !self.can_reach_before_opponent(ctx) {
            // If not, transition to Pressing or HoldingLine state
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position,
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl ForwardInterceptingState {
    fn can_reach_before_opponent(&self, ctx: &StateProcessingContext) -> bool {
        // Calculate time for defender to reach interception point
        let interception_point = self.calculate_interception_point(ctx);
        let defender_distance = (interception_point - ctx.player.position).magnitude();
        let defender_speed = ctx.player.skills.physical.pace.max(0.1); // Avoid division by zero
        let defender_time = defender_distance / defender_speed;

        // Find the minimum time for any opponent to reach the interception point
        let opponent_time = ctx
            .players()
            .opponents()
            .all()
            .map(|opponent| {
                let player = ctx.player();
                let skills = player.skills(opponent.id);

                let opponent_speed = skills.physical.pace.max(0.1);
                let opponent_distance = (interception_point - opponent.position).magnitude();
                opponent_distance / opponent_speed
            })
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(f32::MAX);

        // Return true if defender can reach before any opponent
        defender_time < opponent_time
    }

    /// Calculates the interception point of the ball
    fn calculate_interception_point(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Get ball position and velocity
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        // Defender's speed
        let defender_speed = ctx.player.skills.physical.pace.max(0.1);

        // Relative position and velocity
        let relative_position = ball_position - ctx.player.position;
        let relative_velocity = ball_velocity;

        // Time to intercept
        let time_to_intercept = relative_position.magnitude()
            / (defender_speed + relative_velocity.magnitude()).max(0.1);

        // Predict ball position after time_to_intercept
        ball_position + ball_velocity * time_to_intercept
    }
}
