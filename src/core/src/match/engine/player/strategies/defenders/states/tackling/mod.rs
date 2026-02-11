use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::events::Event;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use rand::{Rng, RngExt};

const TACKLE_DISTANCE_THRESHOLD: f32 = 15.0;
const FOUL_CHANCE_BASE: f32 = 0.2;
const PRESSING_DISTANCE: f32 = 70.0;
const RETURN_DISTANCE: f32 = 100.0;

#[derive(Default)]
pub struct DefenderTacklingState {}

impl StateProcessingHandler for DefenderTacklingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If we have the ball or our team controls it, transition to running
        if ctx.player.has_ball(ctx) || ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // CRITICAL: Don't try to claim ball if it's in protected flight state
        // This prevents the flapping issue where two players repeatedly claim
        if ctx.ball().is_in_flight() {
            return None;
        }

        // Check if there's an opponent with the ball
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            // If opponent is too far for tackling, press instead
            if distance_to_opponent > PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // If opponent is close but not in tackle range, keep pressing
            if distance_to_opponent > TACKLE_DISTANCE_THRESHOLD {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // We're close enough to tackle!
            let (tackle_success, committed_foul) = self.attempt_sliding_tackle(ctx, &opponent);

            return if tackle_success {
                Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::GainBall(ctx.player.id)),
                ))
            } else if committed_foul {
                Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Standing,
                    Event::PlayerEvent(PlayerEvent::CommitFoul),
                ))
            } else {
                None
            };
        } else {
            // Ball is loose - check for interception
            // Double-check not in flight before claiming
            if self.can_intercept_ball(ctx) && !ctx.ball().is_in_flight() {
                // Ball is loose and we can intercept it
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If ball is too far away and not coming toward us, return to position
            let ball_distance = ctx.ball().distance();
            if ball_distance > RETURN_DISTANCE && !ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Returning,
                ));
            }

            // Fallback: if ball is loose and very close, try to claim it
            // Double-check not in flight before claiming
            if !ctx.tick_context.ball.is_owned && ball_distance < 5.0 && !ctx.ball().is_in_flight() {
                return Some(StateChangeResult::with_defender_state_and_event(
                    DefenderState::Running,
                    Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id)),
                ));
            }

            // If opponent is near the player but doesn't have the ball, maybe it's better to transition to pressing
            if let Some(close_opponent) = ctx.players().opponents().nearby(15.0).next() {
                if close_opponent.distance(ctx) < 10.0 {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }
        }

        if ctx.in_state_time > 30 {
            let ball_distance = ctx.ball().distance();
            if ball_distance > PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(DefenderState::Returning));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target = self.calculate_intelligent_target(ctx);

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: Vector3::zeros(), // Static/calculated target position
            }
                .calculate(ctx.player)
                .velocity
                + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Tackling is explosive and very demanding physically
        DefenderCondition::new(ActivityIntensity::VeryHigh).process(ctx);
    }
}

impl DefenderTacklingState {
    fn calculate_intelligent_target(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let own_goal_position = ctx.ball().direction_to_own_goal();

        // Check if ball is dangerously close to own goal
        let ball_distance_to_own_goal = (ball_position - own_goal_position).magnitude();
        let is_ball_near_own_goal = ball_distance_to_own_goal < ctx.context.field_size.width as f32 * 0.2;

        // Check if we're between the ball and our goal
        let player_distance_to_own_goal = (player_position - own_goal_position).magnitude();
        let is_player_closer_to_goal = player_distance_to_own_goal < ball_distance_to_own_goal;

        if is_ball_near_own_goal && !is_player_closer_to_goal {
            // If ball is near our goal and we're not between ball and goal,
            // position ourselves between the ball and the goal
            let ball_to_goal_direction = (own_goal_position - ball_position).normalize();
            let intercept_distance = 5.0; // Stand 5 units in front of the ball towards our goal
            ball_position + ball_to_goal_direction * intercept_distance
        } else {
            // Otherwise, pursue the ball directly
            ball_position
        }
    }

    fn attempt_sliding_tackle(
        &self,
        ctx: &StateProcessingContext,
        opponent: &MatchPlayerLite,
    ) -> (bool, bool) {
        let mut rng = rand::rng();

        let tackling_skill = ctx.player.skills.technical.tackling / 20.0;
        let aggression = ctx.player.skills.mental.aggression / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let overall_skill = (tackling_skill + composure) / 2.0;

        let opponent_dribbling = ctx.player().skills(opponent.id).technical.dribbling / 20.0;
        let opponent_agility = ctx.player().skills(opponent.id).physical.agility / 20.0;

        let skill_difference = overall_skill - (opponent_dribbling + opponent_agility) / 2.0;

        let success_chance = 0.5 + skill_difference * 0.3;
        let clamped_success_chance = success_chance.clamp(0.1, 0.9);

        let tackle_success = rng.random::<f32>() < clamped_success_chance;

        let foul_chance = if tackle_success {
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.05
        } else {
            (1.0 - overall_skill) * FOUL_CHANCE_BASE + aggression * 0.15
        };

        let committed_foul = rng.random::<f32>() < foul_chance;

        (tackle_success, committed_foul)
    }

    fn exists_nearby(&self, ctx: &StateProcessingContext) -> bool {
        const DISTANCE: f32 = 30.0;

        ctx.players().opponents().exists(DISTANCE) || ctx.players().teammates().exists(DISTANCE)
    }

    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        if self.exists_nearby(ctx) {
            return false;
        }

        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let player_position = ctx.player.position;
        let player_speed = ctx.player.skills.physical.pace;

        if !ctx.tick_context.ball.is_owned && ball_velocity.magnitude() > 0.1 {
            let time_to_ball = (ball_position - player_position).magnitude() / player_speed;
            let ball_travel_distance = ball_velocity.magnitude() * time_to_ball;
            let ball_intercept_position =
                ball_position + ball_velocity.normalize() * ball_travel_distance;
            let player_intercept_distance = (ball_intercept_position - player_position).magnitude();

            player_intercept_distance <= TACKLE_DISTANCE_THRESHOLD
        } else {
            false
        }
    }
}
