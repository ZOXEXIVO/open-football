use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const TACKLING_DISTANCE_THRESHOLD: f32 = 3.0; // Distance within which the defender can tackle
const PRESSING_DISTANCE_THRESHOLD: f32 = 50.0; // Max distance to consider pressing
const PRESSING_DISTANCE_DEFENSIVE_THIRD: f32 = 35.0; // Tighter in defensive third
const STAMINA_THRESHOLD: f32 = 40.0; // Increased from 30.0 - prevent overexertion
const FIELD_THIRD_THRESHOLD: f32 = 0.33;

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
                    DefenderState::Tackling,
                ));
            }

            // Context-aware pressing distance: tighter in defensive third
            let pressing_threshold = if ctx.ball().on_own_side()
                && ctx.ball().distance_to_own_goal() < ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD {
                PRESSING_DISTANCE_DEFENSIVE_THIRD
            } else {
                PRESSING_DISTANCE_THRESHOLD
            };

            // 5. If the opponent is too far away, stop pressing
            if distance_to_opponent > pressing_threshold {
                // Transition back to HoldingLine or appropriate state
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            // 6. Check if pressing is creating dangerous gaps
            if self.is_creating_dangerous_gap(ctx, opponent.position) {
                // Drop back to maintain defensive shape
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            // 7. Continue pressing (no state change)
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

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pressing is very demanding - high intensity chasing and pressure
        DefenderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl DefenderPressingState {
    /// Check if pressing is creating a dangerous gap in the defensive line
    fn is_creating_dangerous_gap(&self, ctx: &StateProcessingContext, _opponent_pos: Vector3<f32>) -> bool {
        let own_goal = ctx.ball().direction_to_own_goal();
        let player_pos = ctx.player.position;

        // Get all defenders
        let defenders: Vec<_> = ctx.players().teammates().defenders().collect();

        if defenders.is_empty() {
            return false;
        }

        // Calculate the average defensive line position
        let avg_defender_x = defenders.iter().map(|d| d.position.x).sum::<f32>() / defenders.len() as f32;

        // Check if this defender is significantly ahead of the defensive line
        let distance_ahead_of_line = if own_goal.x < ctx.context.field_size.width as f32 / 2.0 {
            avg_defender_x - player_pos.x // Defending left side
        } else {
            player_pos.x - avg_defender_x // Defending right side
        };

        // If more than 25m ahead of the line, might be creating a gap
        if distance_ahead_of_line > 25.0 {
            // Check if there are dangerous opponents behind the presser
            let dangerous_opponents = ctx.players()
                .opponents()
                .nearby(70.0)
                .filter(|opp| {
                    // Check if opponent is between presser and goal
                    let opp_distance_to_goal = (opp.position - own_goal).magnitude();
                    let presser_distance_to_goal = (player_pos - own_goal).magnitude();

                    opp_distance_to_goal < presser_distance_to_goal
                })
                .count();

            // If there are 2+ dangerous opponents behind, pressing creates a gap
            return dangerous_opponents >= 2;
        }

        false
    }
}
