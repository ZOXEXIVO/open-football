use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const TACKLING_DISTANCE_THRESHOLD: f32 = 2.0; // Distance within which the defender can tackle
const PRESSING_DISTANCE_THRESHOLD: f32 = 20.0; // Max distance to consider pressing
const STAMINA_THRESHOLD: f32 = 30.0; // Minimum stamina to continue pressing

#[derive(Default)]
pub struct DefenderPressingState {}

impl StateProcessingHandler for DefenderPressingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the defender has enough stamina to continue pressing
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            // Transition to Resting state if stamina is low
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // 2. Identify the opponent player with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // 4. If close enough to tackle, transition to Tackling state
            if distance_to_opponent < TACKLING_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::SlidingTackle,
                ));
            }

            // 5. If the opponent is too far away, stop pressing
            if distance_to_opponent > PRESSING_DISTANCE_THRESHOLD {
                // Transition back to HoldingLine or appropriate state
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            // 6. Continue pressing (no state change)
            None
        } else {
            // No opponent with the ball found (perhaps ball is free)
            // Transition back to appropriate state
            Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the opponent with the ball

        let opponents = ctx.players().opponents();
        let mut opponent_with_ball = opponents.with_ball();

        if let Some(opponent) = opponent_with_ball.next() {
            // Calculate direction towards the opponent
            let direction = (opponent.position - ctx.player.position).normalize();
            // Set speed based on player's acceleration and pace
            let speed = ctx.player.skills.physical.pace; // Use pace attribute
            
            return Some(direction * speed + ctx.player().separation_velocity());
        } 
        
        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}
