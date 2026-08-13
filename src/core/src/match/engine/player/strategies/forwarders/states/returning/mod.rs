use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, InterceptionRange,
};
use crate::r#match::{
    ConditionContext, MATCH_TIME_MS, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardReturningState {}

impl StateProcessingHandler for ForwardReturningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
            ));
        }

        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // A LOOSE ball is not a defensive transition.
        //
        // `is_control_ball()` is false both when the opposition have the
        // ball and when NOBODY does, and this state — along with the
        // branch in `Running` that sends players here — read that as
        // "keep running home". Measured off a replay dump: **60% of a
        // forward's time in `Returning` was spent with the ball loose**,
        // sprinting back toward his own start position because nobody
        // happened to be holding it. That is a fifth of a forward's match
        // (22% of his time is in this state) and it is why there is
        // nobody ahead of the ball when the attack rebuilds.
        //
        // Contesting a loose ball is already handled above — Tackling
        // inside 100u, Intercepting inside `COMMIT`. Beyond that range a
        // forward holds his attacking shape; he does not retreat because
        // the ball is bouncing.
        //
        // Offside is the exception: a man stranded past the line has to
        // keep coming back whatever the ball is doing, or he leaves only
        // to be flagged on the next through-ball. Same rule the
        // midfielder state applies.
        if ctx.players().opponents().with_ball().next().is_none()
            && !ctx.player().defensive().is_stranded_offside()
        {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Narrower check first — the old order put the 200u Intercepting
        // branch above this one, so the close-range Tackling branch was
        // unreachable.
        if !ctx.team().is_control_ball() && ctx.ball().distance() < 100.0 {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Tackling,
            ));
        }

        // Commit at the INNER band — `Intercepting` gives up at the outer
        // one, so the two can never both be true (see `InterceptionRange`).
        if !ctx.team().is_control_ball() && ctx.ball().distance() < InterceptionRange::COMMIT {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Back in position. This asked for 2.0 units — **25 centimetres**
        // — before he was allowed to stop, and `Arrive` decelerates into
        // its target, so the last stretch was walked at a crawl. A player
        // is in position within a couple of metres, not to the nearest
        // pixel; the tight number is a large part of why this state was
        // so sticky. 15u = 1.9 m, and the velocity fn holds still inside
        // it so he cannot creep.
        const IN_POSITION: f32 = 15.0;
        if ctx.player().distance_from_start_position() < IN_POSITION {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Standing,
            ));
        }

        // Intercept if ball coming towards player and is closer than before.
        // Bounded by the same give-up range: without the bound this
        // branch re-entered `Intercepting` for a ball the intercepting
        // state would abandon on its very next tick, which is the same
        // two-cycle from a different door.
        if !ctx.team().is_control_ball()
            && ctx.ball().distance() < InterceptionRange::GIVE_UP
            && ctx.ball().is_towards_player_with_angle(0.9)
        {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Transition to Pressing late in the game only if ball is close
        // as well. Final ~1/15 of the match (~6 min full length) — the
        // old `- 180` was 180 ms, so this never fired.
        if ctx.team().is_loosing()
            && ctx.context.total_match_time > MATCH_TIME_MS - MATCH_TIME_MS / 15
            && ctx.ball().distance() < 30.0
        {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player.start_position,
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Returning is moderate intensity - getting back to position
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
