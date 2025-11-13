use nalgebra::Vector3;

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};

const INTERCEPTION_DISTANCE: f32 = 200.0;
const CLEARING_DISTANCE: f32 = 50.0;
const STANDING_TIME_LIMIT: u64 = 300;
const WALK_DISTANCE_THRESHOLD: f32 = 15.0;
const MARKING_DISTANCE: f32 = 15.0;
const FIELD_THIRD_THRESHOLD: f32 = 0.33;
const PRESSING_DISTANCE: f32 = 45.0; // Reduced from 100.0 - more realistic press trigger
const PRESSING_DISTANCE_DEFENSIVE_THIRD: f32 = 35.0; // Even tighter in own defensive third
const TACKLE_DISTANCE: f32 = 30.0;
const BLOCKING_DISTANCE: f32 = 15.0;
const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;
const THREAT_SCAN_DISTANCE: f32 = 70.0; // Extended range for detecting dangerous runs
const DANGEROUS_RUN_SPEED: f32 = 3.0; // Minimum speed to consider a run dangerous
const DANGEROUS_RUN_ANGLE: f32 = 0.7; // Dot product threshold for running toward goal

#[derive(Default)]
pub struct DefenderStandingState {}

impl StateProcessingHandler for DefenderStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();

        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Emergency: if ball is nearby, stopped, and unowned, go for it immediately
        if ball_ops.should_take_ball_immediately() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // Check for nearby opponents with the ball - press them aggressively
        if let Some(opponent) = ctx.players().opponents().with_ball().next() {
            let distance_to_opponent = opponent.distance(ctx);

            if distance_to_opponent < TACKLE_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            // Context-aware pressing distance: tighter in defensive third
            let pressing_threshold = if ctx.ball().on_own_side()
                && ctx.ball().distance_to_own_goal() < ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD {
                PRESSING_DISTANCE_DEFENSIVE_THIRD
            } else {
                PRESSING_DISTANCE
            };

            if distance_to_opponent < pressing_threshold {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }
        }

        // Check for aerial balls requiring heading
        if self.should_head_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Heading,
            ));
        }

        // Check for shots requiring blocking
        if self.should_block_shot(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Blocking,
            ));
        }

        // Check for ball interception opportunities
        if ctx.ball().on_own_side() {
            if ball_ops.is_towards_player_with_angle(0.8)
                && ball_ops.distance() < INTERCEPTION_DISTANCE
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }

            // Only press if opponent has the ball, not just if team doesn't have control
            if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
                if ball_ops.distance() < PRESSING_DISTANCE {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }
        }

        if self.should_push_up(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::PushingUp,
            ));
        }

        if self.should_hold_defensive_line(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // Check for dangerous runs before covering space
        // This ensures defenders pick up attacking threats early
        if let Some(_dangerous_runner) = self.scan_for_dangerous_runs(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Marking,
            ));
        }

        if self.should_cover_space(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Covering,
            ));
        }

        // Walk or hold line more readily on attacking side
        if self.should_transition_to_walking(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Walking,
            ));
        }
        if ctx.in_state_time > 30 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Walking,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Check if player should follow waypoints even when standing
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: 3.0,
                    }
                    .calculate(ctx.player)
                    .velocity * 0.5, // Slower speed when standing
                );
            }
        }

        Some(Vector3::zeros())
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Standing still allows for condition recovery
        DefenderCondition::with_velocity(ActivityIntensity::Recovery).process(ctx);
    }
}

impl DefenderStandingState {
    fn should_transition_to_walking(&self, ctx: &StateProcessingContext) -> bool {
        let player_ops = ctx.player();
        let ball_ops = ctx.ball();

        let is_tired = player_ops.is_tired();
        let standing_too_long = ctx.in_state_time > STANDING_TIME_LIMIT;
        let ball_far_away = ball_ops.distance() > INTERCEPTION_DISTANCE * 2.0;

        // Fixed: inverted logic - should check if there are NO nearby threats
        let no_immediate_threat = ctx
            .players()
            .opponents()
            .nearby(CLEARING_DISTANCE)
            .next()
            .is_none();

        let close_to_optimal_position =
            player_ops.distance_from_start_position() < WALK_DISTANCE_THRESHOLD;
        let team_in_control = ctx.team().is_control_ball();

        (is_tired || standing_too_long)
            && (ball_far_away || close_to_optimal_position)
            && no_immediate_threat
            && team_in_control
    }

    fn should_push_up(&self, ctx: &StateProcessingContext) -> bool {
        let ball_ops = ctx.ball();
        let player_ops = ctx.player();

        let ball_in_attacking_third = ball_ops.distance_to_opponent_goal()
            < ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD;
        let team_in_possession = ctx.team().is_control_ball();
        let defender_not_last_man = !self.is_last_defender(ctx);

        ball_in_attacking_third
            && team_in_possession
            && defender_not_last_man
            && player_ops.distance_from_start_position()
                < ctx.context.field_size.width as f32 * 0.25
    }

    fn should_hold_defensive_line(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().defensive().should_hold_defensive_line()
    }

    fn should_cover_space(&self, ctx: &StateProcessingContext) -> bool {
        let ball_ops = ctx.ball();
        let player_ops = ctx.player();

        let ball_in_middle_third = ball_ops.distance_to_opponent_goal()
            > ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD
            && ball_ops.distance_to_own_goal()
                > ctx.context.field_size.width as f32 * FIELD_THIRD_THRESHOLD;

        // Check for both immediate threats AND dangerous runs
        let no_immediate_threat = ctx
            .players()
            .opponents()
            .nearby(MARKING_DISTANCE)
            .next()
            .is_none();

        let no_dangerous_runs = self.scan_for_dangerous_runs(ctx).is_none();

        let not_in_optimal_position =
            player_ops.distance_from_start_position() > WALK_DISTANCE_THRESHOLD;

        ball_in_middle_third && no_immediate_threat && no_dangerous_runs && not_in_optimal_position
    }

    /// Scan for opponents making dangerous runs toward goal
    fn scan_for_dangerous_runs(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        ctx.player().defensive().scan_for_dangerous_runs()
    }

    fn is_last_defender(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().defensive().is_last_defender()
    }

    fn should_head_ball(&self, ctx: &StateProcessingContext) -> bool {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance = ctx.ball().distance();

        // Ball must be at heading height and close enough
        ball_position.z > HEADING_HEIGHT
            && ball_distance < HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6)
    }

    fn should_block_shot(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // Check if ball is moving fast towards the defender
        if ball_speed < 8.0 || ball_distance > BLOCKING_DISTANCE {
            return false;
        }

        // Check if ball is coming towards player
        if !ctx.ball().is_towards_player_with_angle(0.7) {
            return false;
        }

        // Check if opponent recently shot (ball is fast and low)
        let ball_height = ctx.tick_context.positions.ball.position.z;
        if ball_height > 2.0 {
            return false; // Too high, not a shot
        }

        // Check if there's an opponent nearby who might have shot
        let opponent_nearby = ctx
            .players()
            .opponents()
            .nearby(30.0)
            .any(|opp| opp.has_ball(ctx) || opp.distance(ctx) < 15.0);

        opponent_nearby && ball_distance < BLOCKING_DISTANCE
    }
}
