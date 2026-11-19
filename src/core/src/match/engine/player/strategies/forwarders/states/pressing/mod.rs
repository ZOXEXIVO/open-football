use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardRead, ReadLevel,
};
use crate::r#match::player::strategies::common::states::{
    ChallengeKind, ClosingPoint, RecoveryChallenge, TackleEngagement,
};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **The run to the man on the ball.** Inside challenge range and goal-side
/// of him he contains (`Jockeying`), whatever his tackle cooldown says. He
/// presses while the licence is his; a presser the plan sends with a pass
/// stays on it while the plan names him.
#[derive(Default, Clone)]
pub struct ForwardPressingState {}

impl StateProcessingHandler for ForwardPressingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = ForwardRead::change_above(ctx, ReadLevel::Engage) {
            return Some(change);
        }
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            if TackleEngagement::is_nominated_presser(ctx) && !ctx.team().is_control_ball() {
                return None;
            }
            return Some(ForwardRead::change(ctx));
        };
        if !ForwardRead::is_man_for_carrier(ctx) {
            return Some(ForwardRead::change(ctx));
        }
        if ForwardRead::duel_rung(ctx, &carrier) == ForwardState::Jockeying {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Jockeying,
            ));
        }
        // Gone past him: the stretch from behind is the challenge left.
        let distance = carrier.distance(ctx);
        let lead = RecoveryChallenge::lead(ctx, carrier.position);
        if !MatchContext::forward_duel_off()
            && RecoveryChallenge::is_available(ctx, distance, lead)
            && RecoveryChallenge::is_eligible(ctx)
            && ctx
                .context
                .rng
                .bernoulli(RecoveryChallenge::commits_now(ctx, distance, lead))
        {
            return Some(
                StateChangeResult::with_forward_state(ForwardState::Tackling)
                    .with_staged_challenge(ChallengeKind::Slide),
            );
        }
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if let Some(carrier) = ctx.players().opponents().with_ball().next() {
            let carrier_velocity = ctx.tick_context.positions.players.velocity(carrier.id);
            // A stride off the carrier, across his line to our box — the
            // same point every closer takes (`ClosingPoint`).
            let target = if MatchContext::cross_line_off() {
                ctx.tick_context.positions.ball.position
            } else {
                ClosingPoint::for_carrier(ctx, &carrier)
            };
            let separation = if carrier.distance(ctx) < 25.0 {
                ctx.player().separation_velocity() * 0.05
            } else {
                ctx.player().separation_velocity() * 0.15
            };
            return Some(
                SteeringBehavior::Pursuit {
                    target,
                    target_velocity: carrier_velocity,
                }
                .calculate(ctx.player)
                .velocity
                    + separation,
            );
        }

        // Nobody has it — a pass in flight the plan has sent him with. A
        // press is made on the pass, not after it lands.
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
        // Closing the man on the ball is a sprint: at `High`, 0.78 of top
        // speed, the presser cannot arrive on a carrier running flat out.
        ForwardCondition::new(ActivityIntensity::chase()).process(ctx);
    }
}
