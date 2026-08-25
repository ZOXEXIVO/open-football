use crate::r#match::goalkeepers::states::common::{
    ActivityIntensity, GoalkeeperCondition, KeeperDelivery, KeeperFeetDecision, KeeperOneOnOne,
    KeeperRestPosition, KeeperSmother,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct GoalkeeperReturningGoalState {}

impl StateProcessingHandler for GoalkeeperReturningGoalState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Shot in flight at our goal — stop the jog back and commit
        // to the save.
        if let Some(target) = &ctx.tick_context.ball.cached_shot_target {
            if Some(target.defending_side) == ctx.player.side {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::PreparingForSave,
                ));
            }
        }

        // He is jogging home WITH the ball at his feet. Same question as
        // `Standing` asks, and the same answer: inside his own area with
        // the hands legal, a keeper picks it up rather than dribbling it
        // back and looking for a pass. See [`KeeperFeetDecision`].
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                KeeperFeetDecision::state_for(ctx),
            ));
        }

        // **A keeper does not jog home past a man running at his goal.**
        // Traced on a recording: a carrier three metres away, eleven metres
        // out, and the keeper retreating to his rest point the whole way.
        // See [`KeeperSmother`] and [`KeeperOneOnOne`].
        if let Some(attempt) = KeeperSmother::assess(ctx) {
            return Some(KeeperSmother::commit(ctx, &attempt));
        }
        if KeeperOneOnOne::duel(ctx).is_some() {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        // Loose ball very close — claim it instead of ignoring it. Not the
        // one he has just played, though: jogging home past his own throw
        // and turning round to chase it is the whole of the second report.
        // See [`KeeperDelivery`].
        if !ctx.ball().is_owned()
            && !KeeperDelivery::is_his(ctx)
            && ctx.ball().distance() < 15.0
            && ctx.ball().on_own_side()
        {
            // 5.0 u/tick is above `MAX_SHOT_VELOCITY` (3.2), so this bar
            // excluded nothing and a keeper jogging back gathered shots.
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            if ball_speed < 2.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Catching,
                ));
            }
        }

        // Roughly home is home: `Walking` owns the same rest point and the
        // same tolerance, so the fine adjustment belongs there. Holding on
        // for the tight lateral deadzone instead made this a sticky state
        // — recovery ticks rose 25k → 37k a match for no gain.
        let gap = (ctx.player.position - Self::recovery_point(ctx)).magnitude();
        if gap < KeeperRestPosition::SET_DEADZONE {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Walking,
            ));
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
