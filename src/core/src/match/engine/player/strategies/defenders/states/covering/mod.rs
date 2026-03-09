use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MARKING_DISTANCE: f32 = 25.0; // Increased from 15.0 - pick up attackers earlier
const INTERCEPTION_DISTANCE: f32 = 120.0; // Increased from 100.0
const FIELD_THIRD_THRESHOLD: f32 = 0.33;
const PUSH_UP_HYSTERESIS: f32 = 0.05;
const THREAT_SCAN_DISTANCE: f32 = 100.0; // Increased from 70.0 - wider threat detection
const DANGEROUS_RUN_SPEED: f32 = 2.5; // Reduced from 3.0 - detect slower runs
const DANGEROUS_RUN_ANGLE: f32 = 0.6; // Reduced from 0.7 - wider angle
const MIN_STATE_TIME_DEFAULT: u64 = 20; // Reduced - faster reactions
const MIN_STATE_TIME_WITH_THREAT: u64 = 5; // Reduced - very fast reaction to threats

#[derive(Default, Clone)]
pub struct DefenderCoveringState {}

impl StateProcessingHandler for DefenderCoveringState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Adaptive reaction time based on threat detection
        let min_time = if self.has_dangerous_threat_nearby(ctx) {
            MIN_STATE_TIME_WITH_THREAT
        } else {
            MIN_STATE_TIME_DEFAULT
        };

        if ctx.in_state_time < min_time {
            return None;
        }

        let ball_ops = ctx.ball();

        // Loose ball nearby — go claim it directly
        if !ctx.ball().is_owned() && ctx.ball().distance() < 30.0 && ctx.ball().speed() < 3.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // COUNTER-PRESS: Break from covering to press when possession just lost
        if ctx.team().has_just_lost_possession() {
            let counter_press = ctx.team().tactics().counter_press_intensity();
            if counter_press > 0.4 {
                let ball_dist = ctx.ball().distance();
                let counter_press_range = 30.0 + counter_press * 50.0;
                if ball_dist < counter_press_range {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }
        }

        // Priority: Tackle or press ball carrier aggressively
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let distance = opponent_with_ball.distance(ctx);
            // Very close — tackle immediately regardless
            if distance < 20.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }
            if distance < 50.0 {
                if ctx.player().defensive().is_best_defender_for_opponent(&opponent_with_ball) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
                // Support press if close enough
                if ctx.player().defensive().can_support_press(&opponent_with_ball) {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Pressing,
                    ));
                }
            }
        }

        if ball_ops.on_own_side() {
            // Stay active in covering if opponents are in the area
            // Check both ball carrier AND unmarked attackers making runs
            let ball_carrier_nearby = ctx.players().opponents().with_ball()
                .next()
                .map(|opp| opp.distance(ctx) < 150.0)
                .unwrap_or(false);

            let opponents_in_area = ctx.players().opponents().nearby(80.0).next().is_some();

            if !ball_carrier_nearby && !opponents_in_area {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Standing,
                ));
            }
            // Threats in area — fall through to active covering checks
        }

        if ball_ops.distance_to_opponent_goal()
            < ctx.context.field_size.width as f32 * (FIELD_THIRD_THRESHOLD - PUSH_UP_HYSTERESIS)
            && self.should_push_up(ctx)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::PushingUp,
            ));
        }

        // Look for unmarked dangerous opponents first (coordination)
        if let Some(unmarked) = ctx.player().defensive().find_unmarked_opponent(MARKING_DISTANCE) {
            // Only mark if we're well positioned for this opponent
            if ctx.player().defensive().is_best_defender_for_opponent(&unmarked) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
        }

        // Fall back to basic marking check
        if let Some(_) = ctx.players().opponents().nearby(MARKING_DISTANCE).next() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Marking,
            ));
        }

        // Guard unmarked attackers who are trying to find space
        if let Some(unmarked) = ctx.player().defensive().find_unmarked_opponent(MARKING_DISTANCE * 3.0) {
            if !unmarked.has_ball(ctx) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Guarding,
                ));
            }
        }

        if ball_ops.is_towards_player() && ball_ops.distance() < INTERCEPTION_DISTANCE {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // When an opponent with ball is advancing on own side, position between them and goal
        if ctx.ball().on_own_side() {
            if let Some(opponent) = ctx.players().opponents().with_ball().next() {
                if opponent.distance(ctx) < 120.0 {
                    let own_goal = ctx.ball().direction_to_own_goal();
                    let opp_pos = opponent.position;
                    let opp_velocity = opponent.velocity(ctx);

                    // Predict where opponent will be shortly
                    let opp_future = opp_pos + opp_velocity * 0.3;

                    // Position goal-side of the attacker, offset toward our zone
                    let to_goal = (own_goal - opp_future).normalize();
                    let cover_point = opp_future + to_goal * 25.0;

                    // Blend with tactical position to avoid all defenders collapsing to same spot
                    let tether = 0.2;
                    let target = cover_point * (1.0 - tether) + ctx.player.start_position * tether;

                    let to_target = target - ctx.player.position;
                    let distance = to_target.magnitude();

                    if distance < 3.0 {
                        return Some(opp_velocity * 0.4 + ctx.player().separation_velocity() * 0.3);
                    }

                    let direction = to_target.normalize();
                    let speed = ctx.player.skills.physical.pace * 0.85;
                    let urgency = (distance / 25.0).clamp(0.6, 1.3);

                    return Some(direction * speed * urgency + ctx.player().separation_velocity() * 0.2);
                }
            }
        }

        let target = self.calculate_optimal_covering_position(ctx);
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        // Project ball movement influence onto covering position
        let ball_influence = ball_velocity * 0.3;

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_influence,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Covering space involves moving to cover gaps - moderate intensity
        DefenderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl DefenderCoveringState {
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

    fn is_last_defender(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().teammates().defenders()
            .all(|d| d.position.x >= ctx.player.position.x)
    }

    fn calculate_optimal_covering_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Calculate the center of the middle third with slight offset towards own goal
        let middle_third_center = Vector3::new(
            field_width * 0.4, // Moved slightly back from 0.5
            field_height * 0.5,
            0.0,
        );

        // Get direction to own goal and normalize it
        let ball_to_goal = (ctx.ball().direction_to_own_goal() - ball_position).normalize();

        // Calculate base covering position with better distance scaling
        let covering_distance = (ball_position - ctx.ball().direction_to_own_goal()).magnitude() * 0.35;
        let covering_position = ball_position + ball_to_goal * covering_distance.min(field_width * 0.3);

        // Apply exponential moving average for position smoothing
        const SMOOTHING_FACTOR: f32 = 0.15; // Adjust this value (0.0 to 1.0) to control smoothing
        let previous_position = ctx.player.position;

        // Check for dangerous spaces that need covering
        let dangerous_space = self.find_dangerous_space(ctx);

        // Calculate blended position with weighted factors
        let target_position = if let Some(danger_pos) = dangerous_space {
            // Prioritize covering dangerous space
            Vector3::new(
                danger_pos.x * 0.5 +
                    covering_position.x * 0.3 +
                    player_position.x * 0.2,
                danger_pos.y * 0.5 +
                    covering_position.y * 0.3 +
                    player_position.y * 0.2,
                0.0,
            )
        } else {
            // Default covering behavior - reduced middle_third bias
            Vector3::new(
                covering_position.x * 0.5 +
                    middle_third_center.x * 0.3 + // Reduced from 0.4
                    player_position.x * 0.2,      // Increased from 0.1
                covering_position.y * 0.5 +
                    middle_third_center.y * 0.3 +
                    player_position.y * 0.2,
                0.0,
            )
        };

        // Apply smoothing between frames
        let smoothed_position = previous_position.lerp(&target_position, SMOOTHING_FACTOR);

        // Ensure the position stays within reasonable bounds
        let max_distance_from_center = field_width * 0.35;
        let position_relative_to_center = smoothed_position - middle_third_center;
        let capped_position = if position_relative_to_center.magnitude() > max_distance_from_center {
            middle_third_center + position_relative_to_center.normalize() * max_distance_from_center
        } else {
            smoothed_position
        };

        // Final boundary check
        Vector3::new(
            capped_position.x.clamp(field_width * 0.1, field_width * 0.7),  // Prevent getting too close to either goal
            capped_position.y.clamp(field_height * 0.1, field_height * 0.9), // Keep away from sidelines
            0.0,
        )
    }

    /// Check if there are dangerous threats nearby that require immediate attention
    fn has_dangerous_threat_nearby(&self, ctx: &StateProcessingContext) -> bool {
        // Check for immediate threats within marking distance
        if ctx.players().opponents().nearby(MARKING_DISTANCE).next().is_some() {
            return true;
        }

        // Check for dangerous runs
        let own_goal_position = ctx.ball().direction_to_own_goal();

        ctx.players()
            .opponents()
            .nearby(THREAT_SCAN_DISTANCE)
            .any(|opp| {
                let velocity = opp.velocity(ctx);
                let speed = velocity.norm();

                if speed < DANGEROUS_RUN_SPEED {
                    return false;
                }

                let to_goal = (own_goal_position - opp.position).normalize();
                let velocity_dir = velocity.normalize();
                let alignment = velocity_dir.dot(&to_goal);

                alignment >= DANGEROUS_RUN_ANGLE
            })
    }

    /// Find dangerous space that needs to be covered (e.g., unmarked attackers in dangerous positions)
    fn find_dangerous_space(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let own_goal_position = ctx.ball().direction_to_own_goal();

        // Find opponents making dangerous runs or in dangerous positions
        let dangerous_opponents: Vec<_> = ctx
            .players()
            .opponents()
            .nearby(THREAT_SCAN_DISTANCE)
            .filter(|opp| {
                let velocity = opp.velocity(ctx);
                let speed = velocity.norm();

                // Either running toward goal OR in a dangerous static position
                if speed >= DANGEROUS_RUN_SPEED {
                    let to_goal = (own_goal_position - opp.position).normalize();
                    let velocity_dir = velocity.normalize();
                    velocity_dir.dot(&to_goal) >= DANGEROUS_RUN_ANGLE
                } else {
                    // Check if in dangerous static position (between ball and goal)
                    let ball_pos = ctx.tick_context.positions.ball.position;
                    let distance_to_goal = (opp.position - own_goal_position).magnitude();
                    let ball_distance_to_goal = (ball_pos - own_goal_position).magnitude();

                    // Opponent is closer to goal than ball and within threatening distance
                    distance_to_goal < ball_distance_to_goal && distance_to_goal < 300.0
                }
            })
            .collect();

        if dangerous_opponents.is_empty() {
            return None;
        }

        // Find the most dangerous opponent's position
        let most_dangerous = dangerous_opponents
            .iter()
            .min_by(|a, b| {
                let dist_a = (a.position - own_goal_position).magnitude();
                let dist_b = (b.position - own_goal_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            })?;

        // Calculate position between the dangerous opponent and our goal
        let direction_to_goal = (own_goal_position - most_dangerous.position).normalize();
        Some(most_dangerous.position + direction_to_goal * 15.0)
    }
}
