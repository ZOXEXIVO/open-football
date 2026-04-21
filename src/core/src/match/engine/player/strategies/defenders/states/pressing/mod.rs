use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::player::strategies::players::DefensiveRole;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const TACKLING_DISTANCE_THRESHOLD: f32 = 25.0; // Commit from ~2.5× the old 20u lunge range — carriers can't brush past at 21u
const BASE_PRESSING_DISTANCE: f32 = 45.0;
const MAX_PRESSING_BONUS: f32 = 35.0; // effective range: 45-80
const BASE_PRESSING_DISTANCE_DEFENSIVE_THIRD: f32 = 40.0;
const MAX_PRESSING_BONUS_DEFENSIVE_THIRD: f32 = 30.0; // effective range: 40-70
const CLOSE_PRESSING_DISTANCE: f32 = 25.0; // Wider close pressing zone for tight approach
const STAMINA_THRESHOLD: f32 = 25.0; // Press until truly exhausted. Lowered
// from 30% to match hysteresis with Resting's 45% crisis re-entry
// gate (see `defenders/resting/mod.rs`) — the 25%–45% band is a
// "stay put, slow walk" zone that prevents Pressing↔Resting flicker
// when a crisis stays active while the defender is exhausted.
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

            // If close enough to tackle, transition to Tackling state.
            // Repeat-tackle prevention lives on the player via
            // `tackle_cooldown` — a single-state cooldown here wouldn't
            // cover the Standing/Running/Covering re-entry paths.
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

            // Role-based coordination: a defender only stays in Pressing
            // while they're still the Primary for the current carrier.
            // When the carrier dribbles past (or another defender gets
            // closer), role flips to Cover/Help/Hold and we drop back —
            // Standing's role block will reassign us to the right state
            // on the next tick.
            match ctx.player().defensive().defensive_role_for_ball_carrier() {
                DefensiveRole::Primary => {
                    // Stay on the carrier — aggressive press continues.
                    None
                }
                DefensiveRole::Cover => Some(StateChangeResult::with_defender_state(
                    DefenderState::Covering,
                )),
                DefensiveRole::Help => Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                )),
                DefensiveRole::Hold => Some(StateChangeResult::with_defender_state(
                    DefenderState::HoldingLine,
                )),
            }
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

            // Intercept the carrier's future position based on their
            // velocity — stops them slipping past because the defender
            // was aiming at where they WERE, not where they're going.
            // Previously this was a 5u goal-side offset which meant a
            // running defender arrived ~5u behind the carrier and kept
            // chasing without ever making contact.
            let opp_velocity = ctx.tick_context.positions.players.velocity(opponent.id);
            let opp_speed = opp_velocity.magnitude();
            let pace = ctx.player.skills.physical.pace;
            let aggression = ctx.player.skills.mental.aggression / 20.0;
            let accel = ctx.player.skills.physical.acceleration / 20.0;
            let press_boost = 1.15 + aggression * 0.25 + accel * 0.25; // 1.15x - 1.65x
            let speed = pace * press_boost;

            // Crude lead time: distance / (our speed). If the carrier is
            // moving quickly, aim ahead along their velocity so we meet
            // them. If they're stationary, aim right at the ball.
            let lead_ticks = if speed > 0.01 {
                (distance_to_opponent / speed).min(30.0)
            } else {
                0.0
            };
            let predicted = opponent.position + opp_velocity * lead_ticks;

            // Bias predicted point toward the goal-side so we close the
            // shooting lane even on chase — the defender wants to be
            // BETWEEN the carrier and our goal, not just on top of them.
            //
            // When the carrier is inside shooting range (<80u from our
            // goal), ramp the goalside bias HARD. This puts the defender
            // squarely in the shot-line, which gives them a real chance
            // to block the strike via `try_block_shot`. Real football:
            // a defender closing down in the box shows the shooter
            // his body and steps along the shot line — he doesn't just
            // run at the ball.
            let own_goal = ctx.ball().direction_to_own_goal();
            let to_own_goal = (own_goal - predicted).normalize();
            let carrier_to_goal = (own_goal - predicted).magnitude();
            let shot_zone_bias = if carrier_to_goal < 80.0 {
                // In shot zone: step 8-12u goal-side so we're actually
                // in the shot corridor. Heavier bias closer to goal.
                let zone_factor = 1.0 - (carrier_to_goal / 80.0).clamp(0.0, 1.0);
                8.0 + zone_factor * 4.0
            } else if opp_speed > 0.1 {
                2.0
            } else {
                0.0
            };
            let intercept_target = predicted + to_own_goal * shot_zone_bias;

            let to_target = intercept_target - ctx.player.position;
            let direction = if to_target.magnitude() > 0.01 {
                to_target.normalize()
            } else {
                Vector3::zeros()
            };

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
