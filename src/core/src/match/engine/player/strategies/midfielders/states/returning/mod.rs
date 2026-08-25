use crate::r#match::player::strategies::common::team::WideChannel;
use crate::r#match::midfielders::states::MidfielderGuardingState;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{
    ActivityIntensity, Interception, MidfielderCondition, ShapeStation,
};
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// How close an assigned man has to be before a recovering midfielder
/// abandons the run to his slot and goes to him instead (~19 m). Same
/// figure the running state and the back line use.
const MARK_RECOVERY_DISTANCE: f32 = 150.0;

#[derive(Default, Clone)]
pub struct MidfielderReturningState {}

impl StateProcessingHandler for MidfielderReturningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
        }

        // CRITICAL: Tackle/press if an opponent has the ball nearby
        if let Some(opponent) = ctx
            .players()
            .opponents()
            .nearby(100.0)
            .with_ball(ctx)
            .next()
        {
            let opponent_distance = (opponent.position - ctx.player.position).magnitude();

            if opponent_distance < 40.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
            if opponent_distance < 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        if !ctx.team().is_control_ball()
            && Interception::is_available(ctx)
            && ctx.ball().distance() < 250.0
            && ctx.ball().is_towards_player_with_angle(0.8)
        {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Guard attackers when the ball is on our side — but only if there
        // is actually somebody to guard.
        //
        // The `in_state_time > 30` gate was added to "prevent
        // Returning↔Guarding flicker when no guard target exists", and it
        // cannot: a time delay slows an oscillation down, it does not stop
        // one. `Guarding`'s answer to "no target" is to hand the player
        // straight back here, so the pair simply ran on a 30-tick period
        // instead of a 1-tick one — still ~2,900 round trips a match
        // (`dev_match trace`). Asking the destination's own question is
        // what removes it; the delay stays as a debounce on top.
        // An ASSIGNED man is not subject to the commit range. The plan
        // has already decided he is this midfielder's problem, and
        // `find_committable_guard_target` only commits inside
        // `GUARD_COMMIT_RANGE` (10 m) — so a runner picked out by the
        // plan from fifteen metres was left alone while his marker jogged
        // back to a slot. Measured: 30% of every marking duty in the game
        // was held by somebody in a recovery state, acting on none of it.
        if ctx.in_state_time > 30 && !ctx.team().is_control_ball() {
            if let Some(man) = ctx.team().my_mark() {
                if (man.position - ctx.player.position).magnitude() < MARK_RECOVERY_DISTANCE {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Guarding,
                    ));
                }
            }
        }
        if ctx.in_state_time > 30
            && !ctx.team().is_control_ball()
            && ctx.ball().on_own_side()
            && MidfielderGuardingState::default()
                .find_committable_guard_target(ctx)
                .is_some()
        {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Guarding,
            ));
        }

        // If team has possession, switch to supporting instead of returning home.
        // Gate on offside: attack-minded midfielders caught past the
        // opposing defensive line must keep returning until they're
        // legal again, or they'll exit Returning only to be flagged
        // offside on the very next through-ball.
        if ctx.team().is_control_ball() && ctx.ball().distance() < 300.0 {
            if ctx.player().defensive().is_stranded_offside() {
                // Stay in Returning — the velocity fn drops us back
                // toward start_position, which is onside by definition.
                return None;
            }
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // Recovery run finished — back to active play. `ShapeStation` is
        // the same predicate `MidfielderRunningState` reads before
        // sending anyone back here, so "home" now means one thing to both
        // states instead of two contradictory ones.
        if !ShapeStation::should_recover(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
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
        MidfielderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
