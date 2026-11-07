use crate::r#match::forwarders::states::ForwardCreatingSpaceState;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardPress,
};
use crate::r#match::player::strategies::common::states::{ClosingPoint, TackleEngagement};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardPressingState {}

impl StateProcessingHandler for ForwardPressingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // Back off during foul protection — don't crowd the free kick
        if ctx.ball().is_in_flight() && ctx.ball().is_owned() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Opponent GK holds ball in hands — can't challenge, retreat
        if ctx.ball().is_held_by_opponent_goalkeeper() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Loose ball nearby — go claim it directly instead of pressing thin air
        if !ctx.ball().is_owned()
            && ctx.ball().distance() < 50.0
            && ctx.ball().speed() < 3.0
            && ctx.team().is_best_player_to_chase_ball()
        {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
            ));
        }

        // Commit to a challenge only at genuine contact range, and only
        // when this forward can actually make one. At the old flat 30u
        // this fed `Tackling` continuously — including throughout the
        // tackle cooldown, which `Tackling` answers by handing him back —
        // and the pair ran ~15k round trips a match with 98.8% of
        // `Forward: Tackling` visits lasting a single tick.
        // `should_commit` takes CARRIER distance. This passed
        // `ball().distance()`, which is 3-D and mixes the ball's METRE z
        // with unit x/y — a different quantity in different units.
        if let Some(carrier) = ctx.players().opponents().with_ball().next()
            && TackleEngagement::should_commit(ctx, carrier.distance(ctx))
        {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Tackling,
            ));
        }

        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(
                if ctx.player.has_ball(ctx) {
                    ForwardState::Running
                } else {
                    ForwardCreatingSpaceState::off_ball_support(ctx)
                },
            ));
        }

        // ── The nominated presser does not give himself up ────────────
        //
        // The three exits below are all tighter than the reach the plan
        // nominates over (`DutyAssigner::PRESS_REACH`, 200u ≈ 25 m), so a
        // forward sent here from 20 m out satisfied his own give-up
        // condition on the tick he arrived — the `COMMIT < DISENGAGE`
        // violation `TackleEngagement` was written to remove, restated in
        // the press. He holds the duty until the plan takes it off him,
        // which is at most one tactical refresh (250 ms) away.
        if ForwardPress::called_off(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_distance = ctx.ball().distance();
        // The man the plan sent runs at the ball from wherever he is,
        // instead of drifting home from inside his own duty — the plan
        // nominates over `DutyAssigner::PRESS_REACH` (200u), well outside
        // both radii below. See the note in `process`.
        let nominated = TackleEngagement::is_nominated_presser(ctx);
        let carrier_reach = if nominated {
            f32::INFINITY
        } else {
            ForwardPress::MAX_BALL_DISTANCE
        };
        let loose_reach = if nominated { f32::INFINITY } else { 80.0 };

        // Only pursue if opponent has the ball and it's within pressing range
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            if ball_distance < carrier_reach {
                // A stride off the carrier, across his line to our box —
                // the same point every closer takes (`ClosingPoint`).
                let target = if MatchContext::cross_line_off() {
                    ctx.tick_context.positions.ball.position
                } else {
                    ClosingPoint::for_carrier(ctx, &opponent)
                };
                return Some(
                    SteeringBehavior::Pursuit {
                        target,
                        target_velocity: ctx.tick_context.positions.ball.velocity,
                    }
                    .calculate(ctx.player)
                    .velocity
                        + ctx.player().separation_velocity(),
                );
            }
        } else if !ctx.ball().is_owned() && ball_distance < loose_reach {
            // Loose ball or a pass in flight — go with it. A press is
            // made ON the pass, not after it lands.
            return Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity(),
            );
        }

        // Ball too far or teammate has it — drift back toward start position
        Some(
            SteeringBehavior::Arrive {
                target: ctx.team().my_anchor(),
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pressing is high intensity - sustained running and pressure
        ForwardCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
