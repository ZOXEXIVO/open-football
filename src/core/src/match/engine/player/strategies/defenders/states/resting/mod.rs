use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const STAMINA_RECOVERY_THRESHOLD: f32 = 90.0;
const BALL_PROXIMITY_THRESHOLD: f32 = 10.0;
const MARKING_DISTANCE_THRESHOLD: f32 = 10.0;
const OPPONENT_THREAT_THRESHOLD: usize = 2;

#[derive(Default)]
pub struct DefenderRestingState {}

impl StateProcessingHandler for DefenderRestingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if player's stamina has recovered
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina >= STAMINA_RECOVERY_THRESHOLD {
            // Transition back to HoldingLine state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // 2. Check if the ball is close
        let ball_distance =
            (ctx.tick_context.positions.ball.position - ctx.player.position).magnitude();
        if ball_distance < BALL_PROXIMITY_THRESHOLD {
            // If the ball is close, check for nearby opponents
            let opponent_nearby = self.is_opponent_nearby(ctx);
            return Some(StateChangeResult::with_defender_state(if opponent_nearby {
                DefenderState::Marking
            } else {
                DefenderState::Intercepting
            }));
        }

        // 3. Check if the team is under threat
        if self.is_team_under_threat(ctx) {
            // Transition to Pressing state to help the team
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // 4. Remain in Resting state
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Defender remains stationary or moves minimally while resting
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Resting provides maximum recovery - dedicated recovery state
        DefenderCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}

impl DefenderRestingState {
    /// Checks if an opponent player is nearby within the MARKING_DISTANCE_THRESHOLD.
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(MARKING_DISTANCE_THRESHOLD)
    }

    /// Determines if the team is under threat based on the number of opponents in the attacking third.
    fn is_team_under_threat(&self, ctx: &StateProcessingContext) -> bool {
        let opponents_in_attacking_third = ctx.players().opponents().all()
            .filter(|opponent| self.is_in_defensive_third(opponent.position, ctx))
            .count();

        opponents_in_attacking_third >= OPPONENT_THREAT_THRESHOLD
    }

    /// Checks if a position is within the team's defensive third of the field.
    fn is_in_defensive_third(&self, position: Vector3<f32>, ctx: &StateProcessingContext) -> bool {
        let field_length = ctx.context.field_size.width as f32;
        if ctx.player.side == Some(PlayerSide::Left) {
            position.x < field_length / 3.0
        } else {
            position.x > (2.0 / 3.0) * field_length
        }
    }
}
