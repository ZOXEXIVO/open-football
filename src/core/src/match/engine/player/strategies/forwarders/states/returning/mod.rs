use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::shared::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
use crate::r#match::shared_states::ShapeTempo;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **Getting back to his place**: out of it with no job that keeps him out,
/// or stranded beyond their last line while his side does not have the
/// ball. The read sends him on as soon as anything above that asks for him.
#[derive(Default, Clone)]
pub struct ForwardReturningState {}

impl StateProcessingHandler for ForwardReturningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        ForwardRead::hold(ctx, ForwardState::Returning)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let anchor = ctx.team().my_anchor();
        let to_place = (ctx.player.position - anchor).magnitude();
        if to_place < 8.0 {
            return Some(Vector3::zeros());
        }
        let arrive = SteeringBehavior::Arrive {
            target: anchor,
            slowing_distance: 50.0,
        }
        .calculate(ctx.player)
        .velocity;
        // Separation only while he is still on his way, so it does not
        // fight the arrival.
        if to_place > 30.0 {
            Some(arrive + ctx.player().separation_velocity() * 0.3)
        } else {
            Some(arrive)
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ShapeTempo::forward_declared(
            &ctx,
            ActivityIntensity::Moderate,
        ))
        .process(ctx);
    }
}
