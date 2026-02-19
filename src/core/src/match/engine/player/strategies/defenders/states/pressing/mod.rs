use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const TACKLING_DISTANCE_THRESHOLD: f32 = 6.0; // Increased from 5.0 - slightly earlier tackles
const PRESSING_DISTANCE_THRESHOLD: f32 = 65.0; // Increased from 50.0 - more aggressive pressing
const PRESSING_DISTANCE_DEFENSIVE_THIRD: f32 = 55.0; // Increased from 35.0 - tighter in defensive third
const CLOSE_PRESSING_DISTANCE: f32 = 20.0; // Increased from 15.0 - wider close pressing zone
const STAMINA_THRESHOLD: f32 = 35.0; // Reduced from 40.0 - press more aggressively
const FIELD_THIRD_THRESHOLD: f32 = 0.33;

#[derive(Default)]
pub struct DefenderPressingState {}

impl StateProcessingHandler for DefenderPressingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the defender has enough stamina to continue pressing
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // 2. Identify the opponent player with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // If close enough to tackle, transition to Tackling state
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

            // If the opponent is too far away, stop pressing
            if distance_to_opponent > pressing_threshold {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            // COORDINATION: Check if another defender is already pressing and closer
            // If so, check if we can support-press, otherwise drop back
            if !ctx.player().defensive().is_best_defender_for_opponent(&opponent) {
                // Not the best defender — but can we support the press?
                if ctx.player().defensive().can_support_press(&opponent) {
                    // Stay pressing as a support presser
                } else {
                    // Check if there are unmarked threats we should handle instead
                    if let Some(_unmarked) = ctx.player().defensive().find_unmarked_opponent(60.0) {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Marking,
                        ));
                    }
                    // No unmarked threats, drop back to cover
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Covering,
                    ));
                }
            }

            // Check if pressing is creating dangerous gaps
            if self.is_creating_dangerous_gap(ctx, opponent.position) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                ));
            }

            // Check if we would leave space uncovered
            if ctx.player().defensive().would_leave_space_uncovered(&opponent) {
                // Too risky to press - stay back
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Covering,
                ));
            }

            // Continue pressing
            None
        } else {
            // No opponent with the ball - ball might be loose
            // Check if we should intercept
            if !ctx.ball().is_owned() && ctx.ball().distance() < 50.0 && ctx.ball().speed() < 3.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::TakeBall,
                ));
            }
            if ctx.ball().distance() < 60.0 && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }
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
            let distance_to_opponent = opponent.distance(ctx);

            // Calculate direction towards the opponent
            let direction = (opponent.position - ctx.player.position).normalize();
            // Set speed based on player's acceleration and pace
            let speed = ctx.player.skills.physical.pace; // Use pace attribute

            let pressing_velocity = direction * speed;

            // Reduce separation velocity when actively pressing to allow close approach
            // When very close, disable separation entirely to enable tackling
            let separation = if distance_to_opponent < CLOSE_PRESSING_DISTANCE {
                ctx.player().separation_velocity() * 0.05 // Almost no separation when actively pressing
            } else {
                ctx.player().separation_velocity() * 0.15 // Minimal separation when pressing
            };

            return Some(pressing_velocity + separation);
        }

        // Loose ball nearby — pursue it
        if !ctx.ball().is_owned() && ctx.ball().distance() < 80.0 {
            let direction = (ctx.tick_context.positions.ball.position - ctx.player.position).normalize();
            let speed = ctx.player.skills.physical.pace;
            return Some(direction * speed);
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
