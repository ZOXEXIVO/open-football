use crate::r#match::common_states::LooseBallChase;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, DefenderRead, Interception, ReadLevel,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct DefenderInterceptingState {}

impl StateProcessingHandler for DefenderInterceptingState {
    /// Going to meet a ball nobody has. The election naming him sends him
    /// to collect it; a ball that has gone past him or out of his reach, or
    /// that somebody else has, is the read's.
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = DefenderRead::change_above(ctx, ReadLevel::LooseBall) {
            return Some(change);
        }
        let still_his = Interception::is_available(ctx)
            && !(ctx.ball().speed() > 0.5
                && (!ctx.ball().is_towards_player_with_angle(0.7)
                    || ctx.ball().distance() > Interception::GIVE_UP))
            && LooseBallChase::can_contest(ctx);
        if still_his {
            return None;
        }
        Some(DefenderRead::change(ctx))
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // The state's own answer to where the ball can be met — which
        // until now it computed and then ignored. `velocity()` steered
        // at the ball's CURRENT position, while
        // `calculate_interception_point` was consulted only by
        // `can_reach_before_opponent`, so the state's steering and its
        // own idea of where the ball could be reached were unrelated.
        // One point, used for both.
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
        // Reading a pass and going to it is explosive — the ball is
        // travelling and the window is a stride wide. `High` (0.78 of
        // top speed) is a cruise; see the note in
        // `DefenderPressingState` for why the tier is a speed cap.
        DefenderCondition::new(ActivityIntensity::chase()).process(ctx);
    }
}
