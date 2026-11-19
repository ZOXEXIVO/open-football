use crate::r#match::common_states::{
    ChallengeKind, ClosingPoint, TackleDecision, TackleEngagement, TacticalFoul,
};
use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, ForwardRead, ReadLevel,
};
use crate::r#match::player::events::{FoulCommitted, FoulSeverity, FoulSource, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **Containing the man on the ball.** Goal-side of him, on his shoulder,
/// waiting for the moment. The moment is `TackleDecision`'s, rolled every
/// tick he is in contact; the challenge it opens is `Tackling`'s. A carrier
/// who gets away or goes past him is closed down again; one who releases
/// the ball, or a licence that passes to another man, is the read's.
#[derive(Default, Clone)]
pub struct ForwardJockeyingState {}

impl StateProcessingHandler for ForwardJockeyingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = ForwardRead::change_above(ctx, ReadLevel::Engage) {
            return Some(change);
        }
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return Some(ForwardRead::change(ctx));
        };
        if !ForwardRead::is_man_for_carrier(ctx) {
            return Some(ForwardRead::change(ctx));
        }
        let distance = carrier.distance(ctx);
        if distance > TackleEngagement::DISENGAGE || !ForwardRead::is_goal_side(ctx, &carrier) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }
        if TacticalFoul::commits_now(ctx, carrier.position) {
            return Some(StateChangeResult::with_forward_state_and_event(
                ForwardState::HoldingShape,
                Event::PlayerEvent(PlayerEvent::CommitFoul(FoulCommitted {
                    fouler: ctx.player.id,
                    fouled: carrier.id,
                    severity: FoulSeverity::Normal,
                    source: FoulSource::ProfessionalFoul,
                    challenge: None,
                })),
            ));
        }
        // The moment comes only in contact, and only to a man who may
        // challenge again; declining keeps him here, containing.
        if distance <= TackleEngagement::CONTACT
            && ctx.player.can_attempt_tackle()
            && TackleDecision::is_eligible(ctx)
            && ctx
                .context
                .rng
                .bernoulli(TackleDecision::commits_now(ctx, distance))
        {
            return Some(
                StateChangeResult::with_forward_state(ForwardState::Tackling)
                    .with_staged_challenge(ChallengeKind::Block),
            );
        }
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let carrier = ctx.players().opponents().with_ball().next()?;
        let station = if MatchContext::cross_line_off() {
            ClosingPoint::goal_side(ctx, carrier.position, ClosingPoint::STAND_OFF)
        } else {
            ClosingPoint::for_carrier(ctx, &carrier)
        };
        Some(
            SteeringBehavior::Pursuit {
                target: station,
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity
                + ctx.player().separation_velocity() * 0.15,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // A man backing off cannot match a carrier at full stride.
        let tier = if MatchContext::forward_duel_off() {
            ActivityIntensity::VeryHigh
        } else {
            ActivityIntensity::High
        };
        ForwardCondition::new(tier).process(ctx);
    }
}
