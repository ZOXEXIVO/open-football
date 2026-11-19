use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperRead, KeeperRestPosition, ReadLevel,
};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperReturningGoalState {}

impl StateProcessingHandler for GoalkeeperReturningGoalState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // A cross, a loose ball, a man through, a shot: the read, the same
        // as from his rest point.
        if let Some(change) = KeeperRead::above(ctx, ReadLevel::Position).go() {
            return Some(change);
        }

        // Roughly home is home: the phase states own the same rest point and
        // the same tolerance, so the fine adjustment belongs there. Holding on
        // for the tight lateral deadzone instead made this a sticky state
        // — recovery ticks rose 25k → 37k a match for no gain.
        let gap = (ctx.player.position - Self::recovery_point(ctx)).magnitude();
        if gap < KeeperRestPosition::SET_DEADZONE {
            return KeeperRead::from(ctx, ReadLevel::Phase).go();
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: Self::recovery_point(ctx),
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity
                * KeeperRestPosition::pace(
                    ctx.ball().distance(),
                    ctx.context.field_size.width as f32,
                ),
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Returning to goal requires high intensity as goalkeeper moves back quickly
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl GoalkeeperReturningGoalState {
    /// Where "back" is.
    ///
    /// This state steered to `ctx.player.start_position` — the KICKOFF
    /// DOT, a fixed point at the middle of the goal — and exited when he
    /// was within 15u of it. That is a third, implicit copy of the
    /// keeper's positioning model, and the one place it disagrees with the
    /// other two is the place it matters most: a keeper recovering while
    /// the ball is at the far post ran to the CENTRE of his goal and
    /// arrived off the angle, then had to shuffle again from a standing
    /// start. It is also why, coming out of the leash-driven flicker this
    /// pass removed, he was permanently on the wrong side — every abandoned
    /// sweep ended with a run to a spot that ignored where the ball was.
    ///
    /// He recovers to the same place every other keeper state wants him:
    /// [`KeeperRestPosition`]. Recovering IS repositioning, at speed.
    fn recovery_point(ctx: &StateProcessingContext) -> Vector3<f32> {
        KeeperRestPosition::for_keeper(ctx)
    }
}
