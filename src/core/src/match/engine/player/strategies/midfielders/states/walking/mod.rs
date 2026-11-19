use crate::r#match::common_states::ShapeTempo;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **Relaxed: play is far from his place.** On his place, walking; the read
/// sends him anywhere else the moment the play asks for him.
#[derive(Default, Clone)]
pub struct MidfielderWalkingState {}

impl StateProcessingHandler for MidfielderWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        MidfielderRead::hold(ctx, MidfielderState::Walking)
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
                * 0.4,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        MidfielderCondition::new(ShapeTempo::midfielder_declared(
            &ctx,
            ActivityIntensity::Low,
        ))
        .process(ctx);
    }
}
