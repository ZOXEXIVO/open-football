use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
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

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}