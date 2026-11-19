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

/// **Relaxed: play is far away.** On his slot, walking; the read sends him
/// anywhere else the moment the play asks for him.
#[derive(Default, Clone)]
pub struct DefenderWalkingState {}

impl StateProcessingHandler for DefenderWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        DefenderRead::hold(ctx, DefenderState::Walking)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let slot = DefensiveSlot::point(ctx);
        if (slot - ctx.player.position).magnitude() < 5.0 {
            return Some(Vector3::zeros());
        }
        Some(
            SteeringBehavior::Arrive {
                target: slot,
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity
                * 0.45,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        DefenderCondition::new(ShapeTempo::declared(&ctx, ActivityIntensity::Low)).process(ctx);
    }
}
