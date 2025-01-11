use crate::r#match::defenders::states::DefenderState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior, VectorExtensions};
use crate::IntegerUtils;
use nalgebra::Vector3;

const INTERCEPTION_DISTANCE: f32 = 250.0;
const MARKING_DISTANCE: f32 = 50.0;
const PRESSING_DISTANCE: f32 = 30.0;

#[derive(Default)]
pub struct DefenderWalkingState {}

impl StateProcessingHandler for DefenderWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let mut result = StateChangeResult::new();

        // Transition to Intercepting only if ball moving slowly or player very close
        if ctx.ball().distance() < INTERCEPTION_DISTANCE{
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        // Return to position if far away and no immediate threats
        if ctx.player().position_to_distance() != PlayerDistanceFromStartPosition::Small
            && !self.has_nearby_threats(ctx)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // Mark opponent if they have the ball or are very close
        if let Some(opponent_to_mark) = ctx.players().opponents().without_ball().next() {
            if opponent_to_mark.has_ball(ctx)
                || ctx.player.position.distance_to(&opponent_to_mark.position)
                    < MARKING_DISTANCE / 2.0
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Press opponent only if they have the ball and are close
        if let Some(opponent_to_press) = ctx.players().opponents().with_ball().next() {
            if ctx.player.position.distance_to(&opponent_to_press.position) < PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }
        }

        // Check if the ball is moving towards the player and is close
        if ctx.ball().is_towards_player_with_angle(0.8)
            && ctx.ball().distance() < INTERCEPTION_DISTANCE
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        // Check if the defender needs to return to their position
        if ctx.player().position_to_distance() != PlayerDistanceFromStartPosition::Small {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // Check if there's an opponent to mark
        if let Some(opponent_to_mark) = ctx.players().opponents().with_ball().next() {
            if ctx.player.position.distance_to(&opponent_to_mark.position) < MARKING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Adjust position if needed
        let optimal_position = self.calculate_optimal_position(ctx);
        if ctx.player.position.distance_to(&optimal_position) > 2.0 {
            result
                .events
                .add_player_event(PlayerEvent::MovePlayer(ctx.player.id, optimal_position));
            return Some(result);
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // 1. If this is the first tick in the state, initialize wander behavior
        if ctx.in_state_time % 100 == 0 {
            return Some(
                SteeringBehavior::Wander {
                    target: ctx.player.start_position,
                    radius: IntegerUtils::random(5, 15) as f32,
                    jitter: IntegerUtils::random(1, 5) as f32,
                    distance: IntegerUtils::random(10, 20) as f32,
                    angle: IntegerUtils::random(0, 360) as f32,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }

        // Fallback to moving towards optimal position
        let optimal_position = self.calculate_optimal_position(ctx);
        let direction = (optimal_position - ctx.player.position).normalize();

        let walking_speed = (ctx.player.skills.physical.acceleration + ctx.player.skills.physical.stamina) / 2.0 * 0.1;

        let speed = Vector3::new(walking_speed, walking_speed, 0.0).normalize().norm();
        Some(direction * speed)
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}

impl DefenderWalkingState {
    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // This is a simplified calculation. You might want to make it more sophisticated
        // based on team formation, tactics, and the current game situation.
        let team_center = self.calculate_team_center(ctx);
        let ball_position = ctx.tick_context.positions.ball.position;

        // Position between team center and ball, slightly closer to team center
        (team_center * 0.7 + ball_position * 0.3).into()
    }

    fn calculate_team_center(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let players = ctx.players();
        let teammates = players.teammates();
        let all_teammates: Vec<MatchPlayerLite> = teammates.all().collect();

        let sum: Vector3<f32> = all_teammates.iter().map(|p| p.position).sum();
        sum / all_teammates.len() as f32
    }

    fn has_nearby_threats(&self, ctx: &StateProcessingContext) -> bool {
        let threat_distance = 20.0; // Adjust this value as needed

        if ctx.players().opponents().exists(threat_distance){
            return true;
        }

        // Check if the ball is close and moving towards the player
        let ball_distance = ctx.ball().distance();
        let ball_speed = ctx.ball().speed();
        let ball_towards_player = ctx.ball().is_towards_player();

        if ball_distance < threat_distance && ball_speed > 10.0 && ball_towards_player {
            return true;
        }

        false
    }
}
