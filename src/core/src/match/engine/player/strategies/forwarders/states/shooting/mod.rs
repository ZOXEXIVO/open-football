use crate::r#match::events::Event;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardShootingState {}

impl StateProcessingHandler for ForwardShootingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        Some(StateChangeResult::with_forward_state_and_event(
            ForwardState::Standing,
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::new()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().shooting_direction())
                    .build(ctx)
            )),
        ))
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Shooting is very high intensity - explosive action
        ForwardCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}