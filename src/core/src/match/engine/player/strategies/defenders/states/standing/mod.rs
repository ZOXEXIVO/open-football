use nalgebra::Vector3;

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};

const INTERCEPTION_DISTANCE: f32 = 250.0; // React to balls from further out
const CLEARING_DISTANCE: f32 = 50.0;
const STANDING_TIME_LIMIT: u64 = 300;
const WALK_DISTANCE_THRESHOLD: f32 = 15.0;
const MARKING_DISTANCE: f32 = 35.0; // Pick up attackers early
const FIELD_THIRD_THRESHOLD: f32 = 0.33;
const PRESSING_DISTANCE: f32 = 70.0; // Aggressive pressing range
const PRESSING_DISTANCE_DEFENSIVE_THIRD: f32 = 60.0; // Very tight in own third
const TACKLE_DISTANCE: f32 = 40.0; // Engage tackles from further out
const BLOCKING_DISTANCE: f32 = 35.0; // Block shots from wider range
const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;
#[allow(dead_code)]
const THREAT_SCAN_DISTANCE: f32 = 100.0; // Increased from 70.0 - earlier detection of dangerous runs
#[allow(dead_code)]
const DANGEROUS_RUN_SPEED: f32 = 2.5; // Reduced from 3.0 - detect slower dangerous runs too
#[allow(dead_code)]
const DANGEROUS_RUN_ANGLE: f32 = 0.6; // Reduced from 0.7 - wider angle detection

#[derive(Default, Clone)]
pub struct DefenderStandingState {}

impl StateProcessingHandler for DefenderStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();

        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ball_ops.should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // Check for nearby opponents with the ball - press them if we're best positioned
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

            // Only press if we're the best defender for this opponent (coordination)
            if distance_to_opponent < pressing_threshold {
                if ctx.player().defensive().is_best_defender_for_opponent(&opponent) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                } else {
                    // Not the best defender — check if we can support the press
                    if ctx.player().defensive().can_support_press(&opponent) {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Pressing,
                        ));
                    }
                    // Another defender is better positioned - look for unmarked opponents
                    if let Some(_unmarked) = ctx.player().defensive().find_unmarked_opponent(MARKING_DISTANCE * 2.0) {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Marking,
                        ));
                    }
                    // No unmarked opponent but ball is close - provide cover depth
                    if ctx.ball().on_own_side() {
                        return Some(StateChangeResult::with_defender_state(
                            DefenderState::Covering,
                        ));
                    }
                }
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

            // Only press if opponent has the ball AND we're best positioned
            if let Some(opponent) = ctx.players().opponents().with_ball().next() {
                if ball_ops.distance() < PRESSING_DISTANCE
                    && ctx.player().defensive().is_best_defender_for_opponent(&opponent)
                {
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

        // Guard unmarked attackers who are trying to find space
        if ctx.ball().on_own_side() {
            if let Some(_unmarked) = ctx.player().defensive().find_unmarked_opponent(MARKING_DISTANCE * 2.0) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Guarding,
                ));
            }
        }

        if self.should_cover_space(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Covering,
            ));
        }

        // White zone: loose ball on own side within range - go intercept or track back
        if ctx.ball().on_own_side() && !ctx.ball().is_owned() && ctx.ball().distance() < 200.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
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

        // When ball is on own side, actively position between ball and goal
        if ctx.ball().on_own_side() && ctx.ball().distance() < 200.0 {
            let goal_pos = ctx.ball().direction_to_own_goal();
            let ball_pos = ctx.tick_context.positions.ball.position;
            // Position goal-side of the ball — closer to ball when it's near our goal
            let ball_to_goal_dist = (ball_pos - goal_pos).magnitude();
            let field_width = ctx.context.field_size.width as f32;
            let urgency = 1.0 - (ball_to_goal_dist / field_width).clamp(0.0, 1.0);
            // Blend: more toward ball when it's close to goal, more toward midpoint when far
            let target = ball_pos * (0.3 + urgency * 0.3) + goal_pos * (0.7 - urgency * 0.3);
            let to_target = target - ctx.player.position;
            if to_target.magnitude() > 5.0 {
                return Some(to_target.normalize() * (2.0 + urgency * 2.0));
            }
        }

        // Apply separation velocity to spread out from nearby players
        // This prevents huddle formation even while standing
        let separation = ctx.player().separation_velocity();
        if separation.magnitude() > 1.0 {
            return Some(separation * 0.4);
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

        // Ball must be moving with shot-like speed and within blocking range
        // Shot velocity is capped at ~2.0 per tick, so threshold must be low
        if ball_speed < 0.8 || ball_distance > BLOCKING_DISTANCE {
            return false;
        }

        // Ball must not be owned (it's a free ball / shot in flight)
        if ctx.ball().is_owned() {
            return false;
        }

        // Check if ball is coming towards player
        if !ctx.ball().is_towards_player_with_angle(0.5) {
            return false;
        }

        // Check if opponent recently shot (ball is moving and relatively low)
        let ball_height = ctx.tick_context.positions.ball.position.z;
        if ball_height > 3.0 {
            return false; // Too high to block
        }

        true
    }
}
