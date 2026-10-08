use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

/// Condition he rests back up to before rejoining play. Matched to the
/// second-wind point the other roles use: with the tuned fatigue and
/// recovery rates a forward may never reach 90% mid-match.
const STAMINA_RECOVERY_THRESHOLD: f32 = 60.0;
/// 20 m: a play that comes this close wants him whatever his legs say.
const DEMAND_RANGE: f32 = 160.0;
/// Last resort should his condition never recover and the ball never come
/// near him: 30 s, in AI ticks.
const STALL_GUARD_TICKS: u64 = 1500;
const BALL_PROXIMITY_THRESHOLD: f32 = 10.0;

#[derive(Default, Clone)]
pub struct ForwardRestingState {}

impl StateProcessingHandler for ForwardRestingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Stamina recovered enough - get back in the game
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina >= STAMINA_RECOVERY_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(ForwardState::Walking));
        }

        // 2. Ball is very close - must react regardless of fatigue
        if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD {
            if !ctx.ball().is_owned() && ctx.team().is_best_player_to_chase_ball() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::TakeBall,
                ));
            }
            if let Some(carrier) = ctx.players().opponents().with_ball().next()
                && TackleEngagement::should_commit(ctx, carrier.distance(ctx))
            {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Tackling,
                ));
            }
        }

        // 3. The ball wants him: his side is attacking the half he plays
        //    in, or the play has come to him.
        let attack_needs_him = ctx.team().is_control_ball() && !ctx.ball().on_own_side();
        if attack_needs_him || ctx.ball().distance() < DEMAND_RANGE {
            return Some(StateChangeResult::with_forward_state(ForwardState::Walking));
        }

        if ctx.in_state_time > STALL_GUARD_TICKS {
            return Some(StateChangeResult::with_forward_state(ForwardState::Walking));
        }

        // Stay resting
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}
