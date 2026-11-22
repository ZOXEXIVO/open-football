use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::shared::{
    ActivityIntensity, ForwardCondition, ForwardRead,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **Cutting out a ball in flight that is not his side's.** He stays on it
/// while the read still gives it to him — nobody has it, his side is not on
/// it, and he gets there no later than the opposition — and goes where the
/// read says the moment any of that stops being so.
#[derive(Default, Clone)]
pub struct ForwardInterceptingState {}

impl StateProcessingHandler for ForwardInterceptingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        ForwardRead::hold(ctx, ForwardState::Intercepting)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Onto the ball's path by `Intercept`, which leads it; whether he
        // can get there first is the read's question.
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
        // `High`, matching the defender and midfielder interception states,
        // because this is a speed cap and not a mood: `Moderate` held the
        // forward to 0.52 of top speed while `velocity()` above is a
        // full-blooded `Pursuit` onto a moving ball. Reading the pass is
        // the cheap part; getting there is a sprint, and the two players
        // who might beat him to it are both allowed 0.78.
        ForwardCondition::new(ActivityIntensity::High).process(ctx);
    }
}
