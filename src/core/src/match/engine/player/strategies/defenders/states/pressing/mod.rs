use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const TACKLING_DISTANCE_THRESHOLD: f32 = 20.0; // Engage tackles aggressively — close down fast
const BASE_PRESSING_DISTANCE: f32 = 45.0;
const MAX_PRESSING_BONUS: f32 = 35.0; // effective range: 45-80
const BASE_PRESSING_DISTANCE_DEFENSIVE_THIRD: f32 = 40.0;
const MAX_PRESSING_BONUS_DEFENSIVE_THIRD: f32 = 30.0; // effective range: 40-70
const CLOSE_PRESSING_DISTANCE: f32 = 25.0; // Wider close pressing zone for tight approach
const STAMINA_THRESHOLD: f32 = 30.0; // Press until truly exhausted
const FIELD_THIRD_THRESHOLD: f32 = 0.33;

#[derive(Default, Clone)]
pub struct DefenderPressingState {}

impl StateProcessingHandler for DefenderPressingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the defender has enough stamina to continue pressing
        let stamina = ctx.player.player_attributes.condition_percentage() as f32;
        if stamina < STAMINA_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Resting,
            ));
        }

        // 2. Back off during foul protection — don't crowd the free kick
        if ctx.ball().is_in_flight() && ctx.ball().is_owned() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // 3. Identify the opponent player with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // If close enough to tackle, transition to Tackling state
            if distance_to_opponent < TACKLING_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            // Scale pressing distance by tactical intensity
            let intensity = ctx.team().tactics().pressing_intensity();
            let pressing_threshold = if ctx.ball().on_own_side()
                && ctx.ball().distance_to_own_goal() < ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD {
                BASE_PRESSING_DISTANCE_DEFENSIVE_THIRD + MAX_PRESSING_BONUS_DEFENSIVE_THIRD * intensity
            } else {
                BASE_PRESSING_DISTANCE + MAX_PRESSING_BONUS * intensity
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

            // Continue pressing — stay aggressive
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


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the opponent with the ball

        let opponents = ctx.players().opponents();
        let mut opponent_with_ball = opponents.with_ball();

        if let Some(opponent) = opponent_with_ball.next() {
            let distance_to_opponent = opponent.distance(ctx);

            // Smart pressing: cut off the angle between opponent and our goal
            // instead of just chasing the ball carrier directly
            let own_goal = ctx.ball().direction_to_own_goal();
            let opp_to_goal = (own_goal - opponent.position).normalize();
            // Intercept point: slightly goal-side of opponent
            let intercept_offset = 5.0_f32.min(distance_to_opponent * 0.3);
            let intercept_target = opponent.position + opp_to_goal * intercept_offset;
            let direction = (intercept_target - ctx.player.position).normalize();
            let pace = ctx.player.skills.physical.pace;
            // Defenders press with urgency — acceleration and aggression drive closing speed
            let aggression = ctx.player.skills.mental.aggression / 20.0;
            let accel = ctx.player.skills.physical.acceleration / 20.0;
            let press_boost = 1.1 + aggression * 0.2 + accel * 0.2; // 1.1x - 1.5x
            let speed = pace * press_boost;

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

impl DefenderPressingState {}
