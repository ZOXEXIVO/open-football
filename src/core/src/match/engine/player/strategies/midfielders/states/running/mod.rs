use crate::IntegerUtils;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 300.0;
const MIN_SHOOTING_DISTANCE: f32 = 10.0;

#[derive(Default)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            // Quick shooting checks
            let goal_dist = ctx.ball().distance_to_opponent_goal();

            if goal_dist < MAX_SHOOTING_DISTANCE {
                // Simplified clear shot check
                if goal_dist < 100.0 || (goal_dist < 200.0 && !self.has_close_opponent_fast(ctx)) {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Shooting,
                    ));
                }
            }

            // Simple pressure check
            if self.has_close_opponent_fast(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        } else {
            // Without ball - use simpler checks
            if ctx.ball().distance() < 30.0 && !ctx.ball().is_owned() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            // Check every 10 ticks for less critical states
            if ctx.in_state_time % 10 == 0 {
                if !ctx.team().is_control_ball() && ctx.ball().distance() < 100.0 {
                    return Some(StateChangeResult::with_midfielder_state(
                        MidfielderState::Pressing,
                    ));
                }
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Simplified waypoint following
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();
            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 5.0, // Fixed offset instead of random
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity(),
                );
            }
        }

        // Simplified movement calculation
        if ctx.player.has_ball(ctx) {
            Some(self.calculate_simple_ball_movement(ctx))
        } else if ctx.team().is_control_ball() {
            Some(self.calculate_simple_support_movement(ctx))
        } else {
            Some(self.calculate_simple_defensive_movement(ctx))
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderRunningState {
    /// Fast opponent check without iteration
    fn has_close_opponent_fast(&self, ctx: &StateProcessingContext) -> bool {
        // Use the distance closure which is already calculated
        ctx.tick_context.distances.opponents(ctx.player, 15.0).next().is_some()
    }

    /// Simplified ball carrying movement
    fn calculate_simple_ball_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_pos = ctx.player().opponent_goal_position();
        let player_pos = ctx.player.position;

        // Simple decision: move toward goal with slight variation
        let to_goal = (goal_pos - player_pos).normalize();

        // Add small lateral movement based on time for variation
        let lateral = if ctx.in_state_time % 60 < 30 {
            Vector3::new(-to_goal.y * 0.2, to_goal.x * 0.2, 0.0)
        } else {
            Vector3::new(to_goal.y * 0.2, -to_goal.x * 0.2, 0.0)
        };

        let target = player_pos + (to_goal + lateral).normalize() * 40.0;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 20.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    /// Simplified support movement
    fn calculate_simple_support_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let player_pos = ctx.player.position;

        // Simple triangle formation with ball
        let angle = if player_pos.y < ctx.context.field_size.height as f32 / 2.0 {
            -45.0_f32.to_radians()
        } else {
            45.0_f32.to_radians()
        };

        let support_offset = Vector3::new(
            angle.cos() * 30.0,
            angle.sin() * 30.0,
            0.0
        );

        let target = ball_pos + support_offset;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 15.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    /// Simplified defensive movement
    fn calculate_simple_defensive_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Move toward midpoint between ball and starting position
        let ball_pos = ctx.tick_context.positions.ball.position;
        let start_pos = ctx.player.start_position;

        let target = (ball_pos + start_pos) * 0.5;

        SteeringBehavior::Arrive {
            target,
            slowing_distance: 20.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    // Keep only the essential helper methods with optimizations
    fn in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let dist = ctx.ball().distance_to_opponent_goal();
        dist >= MIN_SHOOTING_DISTANCE && dist <= MAX_SHOOTING_DISTANCE
    }

    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Simplified passing logic - just check if under pressure
        self.has_close_opponent_fast(ctx) &&
            ctx.tick_context.distances.teammates(ctx.player, 50.0, 100.0).next().is_some()
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Simple dribble check
        !self.has_close_opponent_fast(ctx) && ctx.player.skills.technical.dribbling > 15.0
    }
}