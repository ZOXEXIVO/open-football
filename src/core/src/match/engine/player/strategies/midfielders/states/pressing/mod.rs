use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::shared::{
    ActivityIntensity, MidfielderCondition, MidfielderRead, ReadLevel,
};
use crate::r#match::player::strategies::shared::states::{
    ChallengeKind, ClosingPoint, RecoveryChallenge, TackleEngagement,
};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **The run to the man on the ball.** Inside challenge range and goal-side
/// of him he contains (`Jockeying`), whatever his tackle cooldown says; a
/// man the carrier has gone past has the stretch from behind left to him.
/// He presses while the licence is his; a presser the plan sends with a
/// pass stays on it while the plan names him.
#[derive(Default, Clone)]
pub struct MidfielderPressingState {}

impl StateProcessingHandler for MidfielderPressingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = MidfielderRead::change_above(ctx, ReadLevel::Engage) {
            return Some(change);
        }
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            if TackleEngagement::is_nominated_presser(ctx) && !ctx.team().is_control_ball() {
                return None;
            }
            return Some(MidfielderRead::change(ctx));
        };
        if !MidfielderRead::is_man_for_carrier(ctx) {
            return Some(MidfielderRead::change(ctx));
        }
        if MidfielderRead::duel_rung(ctx, &carrier) == MidfielderState::Jockeying {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Jockeying,
            ));
        }
        let distance = carrier.distance(ctx);
        let lead = RecoveryChallenge::lead(ctx, carrier.position);
        if !MatchContext::midfielder_duel_off()
            && RecoveryChallenge::is_available(ctx, distance, lead)
            && RecoveryChallenge::is_eligible(ctx)
            && ctx
                .context
                .rng
                .bernoulli(RecoveryChallenge::commits_now(ctx, distance, lead))
        {
            return Some(
                StateChangeResult::with_midfielder_state(MidfielderState::Tackling)
                    .with_staged_challenge(ChallengeKind::Slide),
            );
        }
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if let Some(carrier) = ctx.players().opponents().with_ball().next() {
            let distance_to_carrier = carrier.distance(ctx);

            // The carrier's velocity leads the aim point, so a running
            // attacker is met where he is going rather than chased.
            let carrier_velocity = ctx.tick_context.positions.players.velocity(carrier.id);

            // A stride off the carrier, across his line to our box —
            // the same point every closer takes (`ClosingPoint`).
            let target = if MatchContext::cross_line_off() {
                let goal_side_bias = if carrier_velocity.magnitude() > 0.1 {
                    2.0
                } else {
                    0.0
                };
                ClosingPoint::goal_side(ctx, carrier.position, goal_side_bias)
            } else {
                ClosingPoint::for_carrier(ctx, &carrier)
            };

            let closing = SteeringBehavior::Pursuit {
                target,
                target_velocity: carrier_velocity,
            }
            .calculate(ctx.player)
            .velocity;

            // Little separation while closing, so he can get to the man.
            let separation = if distance_to_carrier < 25.0 {
                ctx.player().separation_velocity() * 0.05
            } else {
                ctx.player().separation_velocity() * 0.15
            };

            return Some(closing + separation);
        }

        // Nobody has it — a pass in flight the plan has sent him with.
        if !ctx.ball().is_owned() {
            return Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity() * 0.15,
            );
        }

        Some(
            SteeringBehavior::Arrive {
                target: ctx.team().my_anchor(),
                slowing_distance: 25.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Closing the man on the ball is a sprint — see the note in
        // `DefenderPressingState`. `High` is 0.78 of top speed against a
        // carrier running at a flat-out ceiling, so the presser could
        // not arrive.
        MidfielderCondition::new(ActivityIntensity::chase()).process(ctx);
    }
}
