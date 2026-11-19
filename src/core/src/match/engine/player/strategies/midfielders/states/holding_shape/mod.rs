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

/// **Alert: the play is in reach of his place.** The same place the relaxed
/// midfielder walks to, held on his toes; the read sends him on a job, into
/// a duel or back to a walk.
#[derive(Default, Clone)]
pub struct MidfielderHoldingShapeState {}

impl StateProcessingHandler for MidfielderHoldingShapeState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        MidfielderRead::hold(ctx, MidfielderState::HoldingShape)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let place = ctx.team().my_anchor();
        if (place - ctx.player.position).magnitude() < 5.0 {
            return Some(Vector3::zeros());
        }
        Some(
            SteeringBehavior::Arrive {
                target: place,
                slowing_distance: 25.0,
            }
            .calculate(ctx.player)
            .velocity
                + ctx.player().separation_velocity() * 0.4,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        MidfielderCondition::new(ShapeTempo::midfielder_declared(
            &ctx,
            ActivityIntensity::High,
        ))
        .process(ctx);
    }
}
