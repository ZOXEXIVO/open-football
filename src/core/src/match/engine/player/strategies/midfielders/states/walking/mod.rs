use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderWalkingState {}

impl StateProcessingHandler for MidfielderWalkingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // CRITICAL: Check for opponent with ball first (highest priority)
        // Using new chaining syntax: nearby(100.0).with_ball(ctx)
        if let Some(opponent) = ctx.players().opponents().nearby(100.0).with_ball(ctx).next() {
            let opponent_distance = (opponent.position - ctx.player.position).magnitude();

            // If opponent with ball is close, tackle immediately
            if opponent_distance < 40.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }

            // If opponent with ball is nearby, press them (already filtered by nearby(100.0))
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // Loose ball nearby — only chase if best positioned teammate
        if ctx.ball().distance() < 50.0 && !ctx.ball().is_owned() {
            let ball_velocity = ctx.tick_context.positions.ball.velocity.norm();
            if ball_velocity < 3.0 && ctx.team().is_best_player_to_chase_ball() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }
        }

        // Notification: take ball only if best positioned
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
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

            // Compute the nearest opponent distance in a single pass — no
            // intermediate Vec.
            let mut any_nearby = false;
            let mut closest_opponent_distance = f32::MAX;
            for opponent in ctx.players().opponents().nearby(150.0) {
                any_nearby = true;
                let distance = ctx.player().distance_to_player(opponent.id);
                if distance < closest_opponent_distance {
                    closest_opponent_distance = distance;
                }
            }

            if any_nearby {
                let ball_distance = ctx.ball().distance();
                if ball_distance < 50.0 && closest_opponent_distance < 50.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Tackling,
                    ));
                }
            }
        }

        // Don't walk when opponent has ball on our side — go press or guard
        if !ctx.team().is_control_ball() && ctx.ball().on_own_side() {
            if ctx.ball().distance() < 150.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Guarding,
            ));
        }

        // Midfielders shouldn't walk for long — get back into the action
        if ctx.in_state_time > 20 {
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
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 5.0,
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            }
        }

        // Walk toward start position at reduced speed — no random jitter
        let to_start = ctx.player.start_position - ctx.player.position;
        let dist = to_start.magnitude();
        if dist < 5.0 {
            return Some(Vector3::zeros());
        }

        Some(
            SteeringBehavior::Arrive {
                target: ctx.player.start_position,
                slowing_distance: 30.0,
            }
                .calculate(ctx.player)
                .velocity * 0.4, // Walking pace
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Walking is low intensity - minimal fatigue
        MidfielderCondition::with_velocity(ActivityIntensity::Low).process(ctx);
    }
}
