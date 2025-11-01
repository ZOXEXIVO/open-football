use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use crate::IntegerUtils;
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderWalkingState {}

impl StateProcessingHandler for MidfielderWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Emergency: if ball is nearby, stopped, and unowned, go for it immediately
        if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
            let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
            if ball_velocity < 1.0 {
                // Ball is stopped or nearly stopped - take it directly
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }
        }

        if ctx.team().is_control_ball() {
            if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 250.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }
        } else {
            if ctx.ball().distance() < 200.0 && ctx.ball().stopped() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Running,
                ));
            }

            if ctx.ball().is_towards_player_with_angle(0.8) {
                if ctx.ball().distance() < 100.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Intercepting,
                    ));
                }

                if ctx.ball().distance() < 150.0 && ctx.ball().is_towards_player_with_angle(0.8) {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
            }

            let nearby_opponents = ctx.players().opponents().nearby(150.0).collect::<Vec<_>>();
            if !nearby_opponents.is_empty() {
                // If there are nearby opponents, assess the situation
                let ball_distance = ctx.ball().distance();

                let mut closest_opponent_distance = f32::MAX;
                for opponent in &nearby_opponents {
                    let distance = ctx.player().distance_to_player(opponent.id);
                    if distance < closest_opponent_distance {
                        closest_opponent_distance = distance;
                    }
                }

                if ball_distance < 50.0 && closest_opponent_distance < 50.0 {
                    // If the ball is close and an opponent is very close, transition to Tackling state
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }
            }
        }

        if ctx.in_state_time > 100 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Impl ement neural network logic if necessary
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                // Player has waypoints defined, follow them
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 5.0, // Some randomness for natural movement
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            }
        }

        Some(
            SteeringBehavior::Wander {
                target: ctx.player.start_position,
                radius: IntegerUtils::random(5, 150) as f32,
                jitter: IntegerUtils::random(0, 2) as f32,
                distance: IntegerUtils::random(10, 250) as f32,
                angle: IntegerUtils::random(0, 110) as f32,
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}
