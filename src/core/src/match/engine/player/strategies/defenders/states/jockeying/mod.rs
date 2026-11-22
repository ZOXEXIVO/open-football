use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::shared::{
    ActivityIntensity, DefenderCondition, DefenderRead, ReadLevel,
};
use crate::r#match::events::Event;
use crate::r#match::player::events::{FoulCommitted, FoulSeverity, FoulSource, PlayerEvent};
use crate::r#match::player::strategies::shared::players::ops::defender_skill::DefenderSkillProfile;
use crate::r#match::shared_states::{
    BlockChallenge, ChallengeKind, ClosingPoint, TackleDecision, TackleEngagement, TacticalFoul,
};
use crate::r#match::{
    ConditionContext, MatchContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// **Containing the man on the ball.** Goal-side of him, on his shoulder,
/// at his pace, waiting for the moment. The moment is `TackleDecision`'s,
/// rolled every tick he is in contact; the challenge it opens is
/// `Tackling`'s. A carrier who gets away or goes past him is closed down
/// again; one who releases the ball, or a licence that passes to another
/// man, is the read's.
#[derive(Default, Clone)]
pub struct DefenderJockeyingState {}

impl StateProcessingHandler for DefenderJockeyingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if let Some(change) = DefenderRead::change_above(ctx, ReadLevel::Engage) {
            return Some(change);
        }
        let Some(carrier) = ctx.players().opponents().with_ball().next() else {
            return Some(DefenderRead::change(ctx));
        };
        if !DefenderRead::is_man_for_carrier(ctx) {
            return Some(DefenderRead::change(ctx));
        }
        let distance = carrier.distance(ctx);
        if distance > TackleEngagement::DISENGAGE || !DefenderRead::is_goal_side(ctx, &carrier) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }
        if TacticalFoul::commits_now(ctx, carrier.position) {
            return Some(StateChangeResult::with_defender_state_and_event(
                DefenderState::HoldingLine,
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
                StateChangeResult::with_defender_state(DefenderState::Tackling)
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
        let def_profile = DefenderSkillProfile::from_ctx(ctx);
        Some(
            SteeringBehavior::Pursuit {
                target: station,
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity
                * if BlockChallenge::duel_off() {
                    def_profile.tackle_speed_boost()
                } else {
                    1.0
                }
                + ctx.player().separation_velocity() * 0.15,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // A man backing off cannot match a carrier at full stride.
        let tier = if BlockChallenge::duel_off() {
            ActivityIntensity::VeryHigh
        } else {
            ActivityIntensity::High
        };
        DefenderCondition::new(tier).process(ctx);
    }
}
