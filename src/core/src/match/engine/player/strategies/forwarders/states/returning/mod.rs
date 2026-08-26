use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, InterceptionRange,
};
use crate::r#match::player::strategies::common::team::WideChannel;
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
            // Straight back out to the touchline rather than via
            // `Running`, which would spend its own tick deciding the
            // same thing.
            if WideChannel::still_mine(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::HoldingWidth,
                ));
            }
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
        //
        // ⚠ ONLY FOR A BALL THAT CAN ACTUALLY BE INTERCEPTED. This used
        // to send a forward after a ball an OPPONENT WAS CARRYING, and
        // `Intercepting` then asked whether he could reach the
        // interception point before any opponent — against a man standing
        // on the ball, whose distance to it is zero. He always lost, so
        // the state handed him to `Pressing`, which sends him back here
        // the moment the ball is beyond its own 120u range. Three states
        // traversed at roughly one tick each, ~5,100 times a match:
        // `Returning -> Intercepting -> Pressing -> Returning`, and the
        // reason all three sat at the top of the shortest-dwell table
        // (99.7% / 97.5% / 74.4% of visits lasting a single AI tick).
        //
        // A ball at an opponent's feet is a PRESS. Going there directly
        // is the same destination two hops earlier.
        if !ctx.team().is_control_ball() && ctx.ball().distance() < InterceptionRange::COMMIT {
            return Some(StateChangeResult::with_forward_state(
                if ctx.ball().is_owned() {
                    ForwardState::Pressing
                } else {
                    ForwardState::Intercepting
                },
            ));
        }

        // Back in position. This asked for 2.0 units — **25 centimetres**
        // — before he was allowed to stop, and `Arrive` decelerates into
        // its target, so the last stretch was walked at a crawl. A player
        // is in position within a couple of metres, not to the nearest
        // pixel; the tight number is a large part of why this state was
        // so sticky. 15u = 1.9 m, and the velocity fn holds still inside
        // it so he cannot creep.
        //
        // "Position" is the team's live anchor, not the kickoff dot — a
        // forward who has jogged back to where he stood at 0:00 while his
        // block has slid 30 m to the left flank is not in position, he is
        // in the place the match started.
        const IN_POSITION: f32 = 15.0;
        if ctx.team().distance_from_anchor() < IN_POSITION {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Standing,
            ));
        }

        // Intercept if ball coming towards player and is closer than before.
        // Bounded by the same give-up range: without the bound this
        // branch re-entered `Intercepting` for a ball the intercepting
        // state would abandon on its very next tick, which is the same
        // two-cycle from a different door.
        // Same rule as the commit branch above: a ball coming toward him
        // is only interceptable if nobody is carrying it.
        if !ctx.team().is_control_ball()
            && !ctx.ball().is_owned()
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
                target: ctx.team().my_anchor(),
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
