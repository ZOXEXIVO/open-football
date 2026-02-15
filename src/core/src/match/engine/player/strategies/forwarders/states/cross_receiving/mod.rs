use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardCrossReceivingState {}

impl StateProcessingHandler for ForwardCrossReceivingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();

        if !ball_ops.is_towards_player_with_angle(0.8) || ctx.ball().distance() > 100.0 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ball_ops.distance() <= 10.0 {
            return Some(StateChangeResult::with_event(Event::PlayerEvent(PlayerEvent::RequestBallReceive(ctx.player.id))));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        Some(
            SteeringBehavior::Pursuit {
                target: ball_position,
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