use crate::r#match::engine::ball::ball::HandlingVerdict;
use crate::r#match::engine::ball::ball::contest::body::KeeperBody;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::shared::{
    ActivityIntensity, GoalkeeperCondition, KeeperBallClaim, KeeperDelivery, KeeperFeetDecision,
    KeeperRead, KeeperSmother, ReadLevel,
};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::player::strategies::players::ops::goalkeeper_skill::GoalkeeperSkillProfile;
use crate::r#match::shared_states::ChallengeKind;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// **On the ground, from the landing until he is back on his feet.**
///
/// He went down in a dive, a smother or a block, and a man on the floor is
/// not a keeper diving: he moves no faster than a scramble, reaches only
/// what he can reach lying there, and cannot dive again until he is up.
/// How long that takes is the outfield recovery model's
/// (`ChallengeKind::Dive`), so a keeper and a sliding defender get up by one
/// rule. With the ball he stays down at least that long and gets up into
/// `HoldingBall`; without it he goes back into the read.
#[derive(Default, Clone)]
pub struct GoalkeeperGettingUpState {}

impl GoalkeeperGettingUpState {
    fn time_down(ctx: &StateProcessingContext) -> u64 {
        ChallengeKind::Dive.recovery_ticks(
            ctx.player.skills.physical.agility,
            ctx.player.skills.physical.balance,
        )
    }

    /// A loose ball he can reach from where he lies: his spread.
    fn in_lying_reach(ctx: &StateProcessingContext) -> bool {
        let prof = GoalkeeperSkillProfile::from_ctx(ctx);
        !ctx.ball().is_owned()
            && ctx.tick_context.positions.ball.position.z < KeeperBody::HANDS_LYING
            && ctx.ball().distance() <= KeeperSmother::reach(&prof)
    }
}

impl StateProcessingHandler for GoalkeeperGettingUpState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let down = ctx.in_state_time < Self::time_down(ctx);
        if ctx.player.has_ball(ctx) {
            // At his feet rather than in his gloves he is lying beside a
            // ball anyone can take off him: he gathers it now, the rule a
            // dive follows.
            if !ctx.tick_context.ball.held_in_hands {
                return Some(StateChangeResult::with_goalkeeper_state(
                    match ctx.ball().handling_verdict() {
                        HandlingVerdict::Legal => GoalkeeperState::PickingUpBall,
                        _ => KeeperFeetDecision::without_hands(ctx),
                    },
                ));
            }
            return (!down)
                .then(|| StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall));
        }
        if down {
            // A shot at a man lying down is the save model's, like any other.
            let gathers = Self::in_lying_reach(ctx)
                && ctx.tick_context.ball.cached_shot_target.is_none()
                && ctx.ball().handling_verdict().is_legal()
                && !ctx.ball().blocked_from_recollecting()
                && !KeeperDelivery::is_his(ctx)
                && KeeperBallClaim::gathers_this_tick(ctx);
            return gathers.then(|| {
                StateChangeResult::with_goalkeeper_state_and_event(
                    GoalkeeperState::GettingUp,
                    Event::PlayerEvent(PlayerEvent::CaughtBall(ctx.player.id)),
                )
            });
        }
        KeeperRead::from(ctx, ReadLevel::Possession).go()
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.in_state_time >= Self::time_down(ctx) || !Self::in_lying_reach(ctx) {
            return Some(Vector3::zeros());
        }
        // A scramble at the ball: `GoalkeeperSpeedContext::Grounded` is what
        // caps it.
        let to_ball = ctx.tick_context.positions.ball.position - ctx.player.position;
        Some(
            Vector3::new(to_ball.x, to_ball.y, 0.0)
                .try_normalize(1e-3)
                .map_or_else(Vector3::zeros, |way| way * ctx.player.skills.physical.pace),
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        GoalkeeperCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
