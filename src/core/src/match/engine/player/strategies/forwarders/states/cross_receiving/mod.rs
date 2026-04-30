use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardCrossReceivingState {}

impl StateProcessingHandler for ForwardCrossReceivingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();

        if !ball_ops.is_towards_player_with_angle(0.8) || ctx.ball().distance() > 100.0 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ball_ops.distance() <= 10.0 {
            // Aerial ball — head it
            if ctx.tick_context.positions.ball.position.z >= 1.5 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Heading));
            }

            // Ground ball — control it
            return Some(StateChangeResult::with_event(Event::PlayerEvent(
                PlayerEvent::RequestBallReceive(ctx.player.id),
            )));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        // For aerial balls, pursue the estimated landing position
        let target = if ball_position.z >= 1.5 && ball_velocity.z < 0.0 {
            // Estimate where the ball will land (simple ballistic: t = -vz/g, then x += vx*t)
            let gravity = 9.81;
            let time_to_land = (-ball_velocity.z / gravity).max(0.0);
            Vector3::new(
                ball_position.x + ball_velocity.x * time_to_land,
                ball_position.y + ball_velocity.y * time_to_land,
                0.0,
            )
        } else {
            ball_position
        };

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_velocity,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Cross receiving is moderate intensity - positioning and timing
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
