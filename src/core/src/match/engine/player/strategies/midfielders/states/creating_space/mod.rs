use crate::r#match::midfielders::states::common::{
    ActivityIntensity, MidfielderCondition, MidfielderRead,
};
use crate::r#match::midfielders::states::{MidfielderRunningState, MidfielderState};
use crate::r#match::player::strategies::common::players::ops::support::SupportOffer;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// **The short outlet**: showing for the ball off the man on it, while the
/// attack plan names him for it.
#[derive(Default, Clone)]
pub struct MidfielderCreatingSpaceState {}

impl StateProcessingHandler for MidfielderCreatingSpaceState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        MidfielderRead::hold(ctx, MidfielderState::CreatingSpace)
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Self::offer(ctx))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Creating space is moderate intensity - tactical movement
        MidfielderCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}

impl MidfielderCreatingSpaceState {
    /// Showing for the ball: where `SupportOffer` puts an outlet for the
    /// man on it, or for the ball itself while it travels. With the ball at
    /// his own feet before the on-ball state takes him, he carries it as
    /// that state does.
    fn offer(ctx: &StateProcessingContext) -> Vector3<f32> {
        if ctx.player.has_ball(ctx) {
            return MidfielderRunningState::default().carry(ctx);
        }
        let from = ctx
            .ball()
            .owner_id()
            .and_then(|id| ctx.context.players.by_id(id))
            .filter(|carrier| carrier.team_id == ctx.player.team_id)
            .map_or(ctx.tick_context.positions.ball.position, |carrier| {
                ctx.tick_context.positions.players.position(carrier.id)
            });
        let target = SupportOffer::target(ctx, from).unwrap_or_else(|| ctx.team().my_anchor());
        if (target - ctx.player.position).magnitude() < 8.0 {
            return Vector3::zeros();
        }
        SteeringBehavior::Arrive {
            target,
            slowing_distance: 15.0,
        }
        .calculate(ctx.player)
        .velocity
    }
}
