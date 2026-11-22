use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::shared_states::ShapeTempo;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **The recovery run with the opposition on the ball**, or the ball loose:
/// back to his place at full effort, taking the carrier on the way if he
/// becomes the man for him.
#[derive(Default, Clone)]
pub struct MidfielderTrackingBackState {}

impl StateProcessingHandler for MidfielderTrackingBackState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        MidfielderRead::hold(ctx, MidfielderState::TrackingBack)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.team().my_anchor(),
                slowing_distance: 15.0,
            }
            .calculate(ctx.player)
            .velocity
                + ctx.player().separation_velocity() * 0.2,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        MidfielderCondition::new(ShapeTempo::midfielder_declared(
            &ctx,
            ActivityIntensity::VeryHigh,
        ))
        .process(ctx);
    }
}
