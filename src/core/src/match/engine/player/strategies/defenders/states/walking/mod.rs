use crate::r#match::defenders::states::DefenderState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior, VectorExtensions};
use crate::IntegerUtils;
use nalgebra::Vector3;

const INTERCEPTION_DISTANCE: f32 = 150.0;
const MARKING_DISTANCE: f32 = 50.0;
const PRESSING_DISTANCE: f32 = 80.0;
const TACKLE_DISTANCE: f32 = 25.0;
const RUNNING_TO_THE_BALL_DISTANCE: f32 = 150.0;

#[derive(Default)]
pub struct DefenderWalkingState {}

impl StateProcessingHandler for DefenderWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let mut result = StateChangeResult::new();

        // Emergency: if ball is nearby, stopped, and unowned, go for it immediately
        if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
            let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
            if ball_velocity < 1.0 {
                // Ball is stopped or nearly stopped - take it directly
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::TakeBall,
                ));
            }
        }

        if ctx.ball().distance() < RUNNING_TO_THE_BALL_DISTANCE {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall
            ));
        }

        // Priority 1: Check for opponents with the ball nearby - be aggressive!
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = ctx.player.position.distance_to(&opponent.position);

            // Tackle if very close
            if distance_to_opponent < TACKLE_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            // Press if nearby
            if distance_to_opponent < PRESSING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }

            // Mark if within marking range
            if distance_to_opponent < MARKING_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Priority 2: Check for nearby opponents without the ball to mark
        if let Some(opponent_to_mark) = ctx.players().opponents().without_ball().next() {
            let distance = ctx.player.position.distance_to(&opponent_to_mark.position);
            if distance < MARKING_DISTANCE / 2.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Priority 3: Intercept ball if it's coming towards player
        if ctx.ball().is_towards_player_with_angle(0.8)
            && ctx.ball().distance() < INTERCEPTION_DISTANCE
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        // Priority 4: Return to position if far away and no immediate threats
        if ctx.player().position_to_distance() != PlayerDistanceFromStartPosition::Small
            && !self.has_nearby_threats(ctx)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        // Priority 5: Adjust position if needed
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
        // Check if player should follow waypoints
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                // Player has waypoints defined, follow them
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 5.0 // Some randomness for natural movement
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            }
        }
        
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
        let all_teammates: Vec<MatchPlayerLite> = ctx.players().teammates().all().collect();

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
