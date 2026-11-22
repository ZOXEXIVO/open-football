use crate::r#match::forwarders::states::shared::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// **Where a forward starts**: at kick-off, coming on, back on his feet. He
/// reads the game on his first tick and goes where it sends him.
#[derive(Default, Clone)]
pub struct ForwardStandingState {}

impl StateProcessingHandler for ForwardStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        Some(ForwardRead::change(ctx))
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}
