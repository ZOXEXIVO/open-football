use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

// Lowered from 90% — with the tuned fatigue/recovery rates a forward
// may never reach 90% mid-match, which left them stuck in Resting
// indefinitely. 60% matches the second-wind point other roles use and
// still forces a real recovery before re-engaging.
const STAMINA_RECOVERY_THRESHOLD: f32 = 60.0;
/// Hard timeout — even below the stamina threshold, after 500 ticks
/// (5 s) the forward walks. Stops them literally standing still if
/// condition recovery stalls near the threshold.
const MAX_REST_TICKS: u64 = 500;
const BALL_PROXIMITY_THRESHOLD: f32 = 10.0;
const OPPONENT_NEARBY_THRESHOLD: f32 = 10.0;

#[derive(Default, Clone)]
pub struct ForwardRestingState {}

impl StateProcessingHandler for ForwardRestingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Stamina recovered enough - get back in the game
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina >= STAMINA_RECOVERY_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Walking,
            ));
        }

        // 2. Ball is very close - must react regardless of fatigue
        if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD {
            if !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::TakeBall,
                ));
            }
            if ctx.players().opponents().exists(OPPONENT_NEARBY_THRESHOLD) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Tackling,
                ));
            }
        }

        // 3. Hard timeout — can't rest forever. Previously the "team
        // under threat" check tried to force an exit but caused
        // Resting ↔ Pressing flickering. A duration cap is cleaner:
        // after 5 s you're jogging regardless of stamina, just at
        // reduced intensity. The fatigue curve will keep penalising
        // their pace so it's still a meaningful rest.
        if ctx.in_state_time > MAX_REST_TICKS {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Walking,
            ));
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

