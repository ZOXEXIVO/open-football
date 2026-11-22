use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, Interception, MidfielderCondition, MidfielderRead, ReadLevel,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **Stepping in to cut out a ball in flight.** Entered by the read alone,
/// for a ball nobody holds that he can reach before an opponent; once it is
/// somebody's, or no longer his to contest, the read decides again.
#[derive(Default, Clone)]
pub struct MidfielderInterceptingState {}

impl StateProcessingHandler for MidfielderInterceptingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = MidfielderRead::change_above(ctx, ReadLevel::LooseBall) {
            return Some(change);
        }
        if !Interception::is_available(ctx) {
            return Some(MidfielderRead::change(ctx));
        }
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Where he can meet the ball: `SteeringBehavior::Intercept` leads
        // the ball's position by its velocity, rather than chasing where
        // it is now.
        Some(
            SteeringBehavior::Intercept {
                target: ctx.tick_context.positions.ball.position,
                target_velocity: ctx.tick_context.positions.ball.velocity,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Reading a pass and going to it is explosive, not a cruise —
        // same tier as the back line's `Intercepting`, and for the same
        // reason (the tier is a speed cap).
        MidfielderCondition::new(ActivityIntensity::chase()).process(ctx);
    }
}
