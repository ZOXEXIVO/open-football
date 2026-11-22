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

/// **Getting back into shape when nothing presses**: his own side has the
/// ball, or it is dead. A jog to his place.
#[derive(Default, Clone)]
pub struct MidfielderReturningState {}

impl StateProcessingHandler for MidfielderReturningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        MidfielderRead::hold(ctx, MidfielderState::Returning)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let anchor = ctx.team().my_anchor();
        let dist_to_start = (ctx.player.position - anchor).magnitude();

        // Close enough — stop to prevent oscillation
        if dist_to_start < 8.0 {
            return Some(Vector3::zeros());
        }

        let arrive = SteeringBehavior::Arrive {
            target: anchor,
            slowing_distance: 50.0,
        }
        .calculate(ctx.player)
        .velocity;

        // Only add separation when far from target — prevents fighting near destination
        if dist_to_start > 30.0 {
            Some(arrive + ctx.player().separation_velocity() * 0.3)
        } else {
            Some(arrive)
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Returning is moderate intensity - getting back to position
        MidfielderCondition::new(ShapeTempo::midfielder_declared(
            &ctx,
            ActivityIntensity::Moderate,
        ))
        .process(ctx);
    }
}
