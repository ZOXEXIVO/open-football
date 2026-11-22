use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// **Where a defender starts**: at kick-off, coming on, back on his feet.
/// He reads the game on his first tick and goes where it sends him.
#[derive(Default, Clone)]
pub struct DefenderStandingState {}

impl StateProcessingHandler for DefenderStandingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        Some(DefenderRead::change(ctx))
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        DefenderCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}
