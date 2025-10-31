use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;

#[derive(Default)]
pub struct DefenderInterceptingState {}

impl StateProcessingHandler for DefenderInterceptingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Check if ball is aerial and at heading height
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance = ctx.ball().distance();

        if ball_position.z > HEADING_HEIGHT
            && ball_distance < HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Heading,
            ));
        }

        if ball_distance < 20.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Tackling,
            ));
        }

        if !ctx.ball().is_towards_player_with_angle(0.8) || ball_distance > 100.0  {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        if !self.can_reach_before_opponent(ctx) {
            // If not, transition to Pressing or HoldingLine state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
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
                target: ctx.tick_context.positions.ball.position,
                target_velocity: ctx.tick_context.positions.ball.velocity,
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl DefenderInterceptingState {
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
        // For aerial balls, use the precalculated landing position
        let ball_position = ctx.tick_context.positions.ball.position;
        let landing_position = ctx.tick_context.positions.ball.landing_position;

        // Check if ball is aerial (high enough that landing position differs significantly)
        let is_aerial = (ball_position - landing_position).magnitude() > 5.0;

        if is_aerial {
            // For aerial balls, target the landing position
            landing_position
        } else {
            // For ground balls, do normal interception calculation
            let ball_velocity = ctx.tick_context.positions.ball.velocity;
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
}
