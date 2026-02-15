use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const STAMINA_RECOVERY_THRESHOLD: f32 = 90.0;
const BALL_PROXIMITY_THRESHOLD: f32 = 10.0;
const OPPONENT_NEARBY_THRESHOLD: f32 = 10.0;
const OPPONENT_THREAT_THRESHOLD: usize = 2;

#[derive(Default)]
pub struct ForwardRestingState {}

impl StateProcessingHandler for ForwardRestingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
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

        // 3. Team is under defensive threat - must help
        if self.is_team_under_threat(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
            ));
        }

        // Stay resting
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        ForwardCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl ForwardRestingState {
    fn is_team_under_threat(&self, ctx: &StateProcessingContext) -> bool {
        let opponents_in_defensive_third = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                let field_length = ctx.context.field_size.width as f32;
                if ctx.player.side == Some(PlayerSide::Left) {
                    opponent.position.x < field_length / 3.0
                } else {
                    opponent.position.x > (2.0 / 3.0) * field_length
                }
            })
            .count();

        opponents_in_defensive_third >= OPPONENT_THREAT_THRESHOLD
    }
}
