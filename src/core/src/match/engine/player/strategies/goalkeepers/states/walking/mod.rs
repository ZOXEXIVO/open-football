use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperRead, KeeperRestPosition, KeeperSetPieceStance,
    ReadLevel,
};
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::player::strategies::processor::StateChangeResult;
use crate::r#match::player::strategies::processor::{
    StateProcessingContext, StateProcessingHandler,
};
use crate::r#match::{ConditionContext, SteeringBehavior};
use nalgebra::Vector3;

/// **Relaxed**: his side has the ball and it is far from his goal. Same rest
/// point and same read as `Standing`; he strolls rather than stands on his
/// toes, which is what the viewer draws.
#[derive(Default, Clone)]
pub struct GoalkeeperWalkingState {}

impl StateProcessingHandler for GoalkeeperWalkingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        KeeperRead::from(ctx, ReadLevel::Possession).go()
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Same resting position as every other keeper state — see
        // `KeeperRestPosition`. This state used to own a SECOND copy of
        // the positioning model with different constants, so the same
        // keeper wanted to be in two different places depending on which
        // state he happened to be in.
        //
        // And within 10u of it he `Wander`ed, on a 6.25 m radius, forever:
        // measured **0% still** in this state, a keeper pacing aimlessly
        // around his box. Same pattern removed from the outfield walking
        // states. He stands set instead.
        //
        // …and a dead ball at his goal has a mark of its own, read ahead
        // of the rest model for the reason `Standing::velocity` gives.
        if let Some(to_mark) = KeeperSetPieceStance::steer(ctx) {
            return Some(to_mark);
        }
        let optimal_position = self.calculate_intelligent_position(ctx);
        if KeeperRestPosition::is_set_with(
            ctx.player.position,
            optimal_position,
            GoalkeeperSkillProfile::from_ctx(ctx).concentration,
            ctx.ball().distance(),
            ctx.context.field_size.width as f32,
        ) {
            return Some(Vector3::zeros());
        }

        let pace =
            KeeperRestPosition::pace(ctx.ball().distance(), ctx.context.field_size.width as f32);
        Some(
            SteeringBehavior::Arrive {
                target: optimal_position,
                slowing_distance: 8.0,
            }
            .calculate(ctx.player)
            .velocity
                * pace,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking state has low intensity but more activity than standing
        GoalkeeperCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}

impl GoalkeeperWalkingState {
    /// The one shared keeper positioning model. See `KeeperRestPosition`.
    fn calculate_intelligent_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        KeeperRestPosition::for_keeper(ctx)
    }
}
