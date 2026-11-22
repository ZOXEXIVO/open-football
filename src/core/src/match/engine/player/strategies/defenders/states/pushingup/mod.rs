use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead, DefensiveSlot,
};
use crate::r#match::shared_states::ShapeTempo;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **Squeezing up behind the ball in possession.** Each defender on his own
/// slot, which moves up with the block; the deepest man moves up with the
/// rest. Winning the ball is the read's first door and losing it its last.
#[derive(Default, Clone)]
pub struct DefenderPushingUpState {}

impl StateProcessingHandler for DefenderPushingUpState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        DefenderRead::hold(ctx, DefenderState::PushingUp)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: DefensiveSlot::point(ctx),
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pushing up involves moving forward with the team - moderate intensity
        DefenderCondition::new(ShapeTempo::declared(&ctx, ActivityIntensity::Moderate))
            .process(ctx);
    }
}
