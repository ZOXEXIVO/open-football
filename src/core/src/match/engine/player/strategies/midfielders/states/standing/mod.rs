use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// **Where a midfielder starts**: at kick-off, coming on, back on his feet.
/// He reads the game on his first tick and goes where it sends him.
#[derive(Default, Clone)]
pub struct MidfielderStandingState {}

impl StateProcessingHandler for MidfielderStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        Some(MidfielderRead::change(ctx))
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        MidfielderCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}
