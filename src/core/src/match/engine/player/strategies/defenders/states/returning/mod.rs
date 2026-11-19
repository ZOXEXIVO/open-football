use crate::r#match::common_states::ShapeTempo;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, DefenderRead, DefensiveSlot,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **Getting back into shape when nothing presses**: his own side has the
/// ball, or it is dead. A jog to his slot.
#[derive(Default, Clone)]
pub struct DefenderReturningState {}

impl StateProcessingHandler for DefenderReturningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        DefenderRead::recovering(ctx, DefenderState::Returning)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: DefensiveSlot::point(ctx),
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Returning to position involves jogging back - moderate intensity
        DefenderCondition::new(ShapeTempo::declared(&ctx, ActivityIntensity::Moderate))
            .process(ctx);
    }
}
