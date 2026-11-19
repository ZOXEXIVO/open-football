use crate::r#match::common_states::ShapeTempo;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// Fraction of full speed a walking player moves at. This is the
/// recovery gear — the state exists so a forward can reorganise without
/// burning condition, so he covers ground but does not sprint.
const WALK_PACE: f32 = 0.45;

/// **Relaxed: play is far from his place.** On his place, walking; the read
/// sends him anywhere else the moment the play asks for him.
#[derive(Default, Clone)]
pub struct ForwardWalkingState {}

impl StateProcessingHandler for ForwardWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        ForwardRead::hold(ctx, ForwardState::Walking)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let place = ctx.team().my_anchor();
        if (place - ctx.player.position).magnitude() < 5.0 {
            return Some(Vector3::zeros());
        }
        Some(
            SteeringBehavior::Arrive {
                target: place,
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity
                * WALK_PACE,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ShapeTempo::forward_declared(
            &ctx,
            ActivityIntensity::Low,
        ))
        .process(ctx);
    }
}
