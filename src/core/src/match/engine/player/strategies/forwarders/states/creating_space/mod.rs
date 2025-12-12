use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};

// Movement patterns for forwards
#[derive(Debug, Clone, Copy)]
enum ForwardMovementPattern {
    DirectRun,        // Direct run behind defense
    DiagonalRun,      // Diagonal run to create space and angles
    ChannelRun,       // Run between defenders
    DriftWide,        // Drift wide to create central space
    CheckToFeet,      // Come short to receive
    OppositeMovement, // Move opposite to defensive shift
}

use nalgebra::Vector3;

const MAX_DISTANCE_FROM_BALL: f32 = 80.0;
const MIN_DISTANCE_FROM_BALL: f32 = 15.0;
const OPTIMAL_PASSING_DISTANCE_MIN: f32 = 15.0; // Wider ideal passing range start (was 20.0)
const OPTIMAL_PASSING_DISTANCE_MAX: f32 = 60.0; // Wider ideal passing range end (was 45.0)
const SPACE_SCAN_RADIUS: f32 = 100.0;
const CONGESTION_THRESHOLD: f32 = 3.0;
const PASSING_LANE_IMPORTANCE: f32 = 15.0; // High weight for clear passing lanes

#[derive(Default)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has the ball
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // Check if team lost possession
        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // If ball is close and moving toward player
        if ctx.ball().distance() < 100.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Check if created good space
        if self.has_created_good_space(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        }

        // Check for forward run opportunity
        if self.should_make_forward_run(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::RunningInBehind,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // PRIORITY: When teammate has ball, make aggressive run toward goal
        if let Some(ball_holder) = self.get_ball_holder(ctx) {
            let holder_pos = ball_holder.position;

            // Calculate attacking direction
            let attacking_direction = match ctx.player.side {
                Some(PlayerSide::Left) => Vector3::new(1.0, 0.0, 0.0),
                Some(PlayerSide::Right) => Vector3::new(-1.0, 0.0, 0.0),
                None => (goal_pos - player_pos).normalize(),
            };

            // Determine if we're ahead of or behind ball holder
            let is_ahead = match ctx.player.side {
                Some(PlayerSide::Left) => player_pos.x > holder_pos.x + 10.0,
                Some(PlayerSide::Right) => player_pos.x < holder_pos.x - 10.0,
                None => false,
            };

            // Calculate target: run TOWARD the goal with lateral spread
            let target_position = if is_ahead {
                // Already ahead - run further toward goal
                let forward_distance = 80.0;
                let lateral_offset = if player_pos.y < field_height / 2.0 {
                    -25.0 // Stay on left side
                } else {
                    25.0 // Stay on right side
                };

                Vector3::new(
                    (player_pos.x + attacking_direction.x * forward_distance)
                        .clamp(50.0, field_width - 50.0),
                    (player_pos.y + lateral_offset).clamp(50.0, field_height - 50.0),
                    0.0,
                )
            } else {
                // Behind ball holder - get ahead of them quickly
                let overtake_distance = 100.0;
                let lateral_spread = if player_pos.y < field_height / 2.0 {
                    -40.0 // Spread to left
                } else {
                    40.0 // Spread to right
                };

                Vector3::new(
                    (holder_pos.x + attacking_direction.x * overtake_distance)
                        .clamp(50.0, field_width - 50.0),
                    (holder_pos.y + lateral_spread).clamp(50.0, field_height - 50.0),
                    0.0,
                )
            };

            // Check for offside - if would be offside, come back slightly
            let final_target = if self.would_be_offside(ctx, target_position) {
                // Stay just onside
                let defensive_line = self.get_defensive_line_position(ctx);
                let safe_x = match ctx.player.side {
                    Some(PlayerSide::Left) => defensive_line - 5.0,
                    Some(PlayerSide::Right) => defensive_line + 5.0,
                    None => defensive_line,
                };
                Vector3::new(safe_x, target_position.y, 0.0)
            } else {
                target_position
            };

            // Use Pursuit for aggressive movement
            let base_velocity = SteeringBehavior::Pursuit {
                target: final_target,
                target_velocity: Vector3::zeros(),
            }
            .calculate(ctx.player)
            .velocity;

            // Add separation to avoid clustering
            return Some(base_velocity + ctx.player().separation_velocity() * 1.5);
        }

        // Fallback: use the existing complex logic when no teammate has ball
        let target_position = self.find_optimal_free_zone(ctx);
        let avoidance_vector = self.calculate_dynamic_avoidance(ctx);
        let movement_pattern = self.get_intelligent_movement_pattern(ctx);

        match movement_pattern {
            ForwardMovementPattern::DirectRun => {
                let base_velocity = SteeringBehavior::Pursuit {
                    target: target_position,
                    target_velocity: Vector3::zeros(),
                }
                    .calculate(ctx.player)
                    .velocity;

                Some(base_velocity + avoidance_vector * 1.2 + ctx.player().separation_velocity())
            }
            ForwardMovementPattern::DiagonalRun => {
                let diagonal_target = self.calculate_diagonal_run_target(ctx, target_position);
                let base_velocity = SteeringBehavior::Arrive {
                    target: diagonal_target,
                    slowing_distance: 15.0,
                }
                    .calculate(ctx.player)
                    .velocity;

                Some(base_velocity + avoidance_vector)
            }
            ForwardMovementPattern::ChannelRun => {
                let channel_target = self.find_defensive_channel(ctx);
                Some(
                    SteeringBehavior::Pursuit {
                        target: channel_target,
                        target_velocity: Vector3::zeros(),
                    }
                        .calculate(ctx.player)
                        .velocity + avoidance_vector * 0.8
                )
            }
            ForwardMovementPattern::DriftWide => {
                let wide_target = self.calculate_wide_position(ctx);
                Some(
                    SteeringBehavior::Arrive {
                        target: wide_target,
                        slowing_distance: 20.0,
                    }
                        .calculate(ctx.player)
                        .velocity + avoidance_vector * 0.6
                )
            }
            ForwardMovementPattern::CheckToFeet => {
                let check_target = self.calculate_check_position(ctx);
                Some(
                    SteeringBehavior::Arrive {
                        target: check_target,
                        slowing_distance: 10.0,
                    }
                        .calculate(ctx.player)
                        .velocity
                )
            }
            ForwardMovementPattern::OppositeMovement => {
                let opposite_target = self.calculate_opposite_movement(ctx);
                Some(
                    SteeringBehavior::Arrive {
                        target: opposite_target,
                        slowing_distance: 15.0,
                    }
                        .calculate(ctx.player)
                        .velocity + avoidance_vector * 1.5
                )
            }
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Creating space is moderate intensity - tactical movement
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl ForwardCreatingSpaceState {
    /// Find optimal free zone for a forward
    /// Find optimal free zone for a forward - optimized to search gaps between opponents
    fn find_optimal_free_zone(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // Collect relevant opponents (defenders and midfielders in forward zones)
        let opponents: Vec<Vector3<f32>> = ctx.players()
            .opponents()
            .all()
            .filter(|opp| {
                let is_relevant_position = opp.tactical_positions.is_defender()
                    || opp.tactical_positions.is_midfielder();
                let distance = (opp.position - player_pos).magnitude();
                is_relevant_position && distance < SPACE_SCAN_RADIUS
            })
            .map(|opp| opp.position)
            .collect();

        // If no nearby opponents, move toward goal
        if opponents.is_empty() {
            let forward_direction = (goal_pos - player_pos).normalize();
            return self.apply_forward_tactical_adjustment(
                ctx,
                player_pos + forward_direction * 30.0,
            );
        }

        // Find gaps between opponents using improved multi-strategy approach
        let mut candidate_positions = Vec::with_capacity(40);

        // Strategy 1: Midpoints between adjacent opponents (existing)
        for i in 0..opponents.len() {
            for j in (i + 1)..opponents.len() {
                let midpoint = (opponents[i] + opponents[j]) * 0.5;
                let gap_width = (opponents[i] - opponents[j]).magnitude();

                // Widened gap consideration (was 15-60, now 12-80)
                if gap_width > 12.0 && gap_width < 80.0 {
                    candidate_positions.push(midpoint);

                    // NEW: Also add positions pushed forward through the gap
                    let to_goal = (goal_pos - midpoint).normalize();
                    candidate_positions.push(midpoint + to_goal * 10.0);
                }
            }
        }

        // Strategy 2: Positions offset from opponents (existing, improved)
        for &opp_pos in &opponents {
            let to_goal = (goal_pos - opp_pos).normalize();
            let perpendicular = Vector3::new(-to_goal.y, to_goal.x, 0.0);

            // Wider lateral positions and deeper runs
            candidate_positions.push(opp_pos + perpendicular * 25.0 + to_goal * 20.0);
            candidate_positions.push(opp_pos - perpendicular * 25.0 + to_goal * 20.0);

            // NEW: Positions directly behind defenders (in space behind)
            candidate_positions.push(opp_pos + to_goal * 15.0);
        }

        // Strategy 3: NEW - Grid-based open space detection
        // Create grid of positions in attacking third and find truly open ones
        let forward_direction = (goal_pos - player_pos).normalize();
        for x_offset in [20.0, 35.0, 50.0] {
            for y_offset in [-30.0, -15.0, 0.0, 15.0, 30.0] {
                let lateral = Vector3::new(-forward_direction.y, forward_direction.x, 0.0);
                let candidate = player_pos + forward_direction * x_offset + lateral * y_offset;
                candidate_positions.push(candidate);
            }
        }

        // Add current player position as fallback
        candidate_positions.push(player_pos);

        // Evaluate candidates and pick best
        let mut best_position = player_pos;
        let mut best_score = f32::MIN;

        for candidate in candidate_positions {
            // Ensure position is within bounds
            let clamped = Vector3::new(
                candidate.x.clamp(20.0, field_width - 20.0),
                candidate.y.clamp(20.0, field_height - 20.0),
                0.0,
            );

            let score = self.evaluate_forward_position(ctx, clamped);

            if score > best_score {
                best_score = score;
                best_position = clamped;
            }
        }

        // Apply tactical adjustments
        self.apply_forward_tactical_adjustment(ctx, best_position)
    }

    /// Evaluate a position for forward play
    fn evaluate_forward_position(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let mut score = 0.0;

        // Space score (inverse of congestion)
        let congestion = self.calculate_position_congestion(ctx, position);
        score += (10.0 - congestion.min(10.0)) * 3.0;

        // Goal threat score
        let goal_threat = self.calculate_goal_threat(ctx, position);
        score += goal_threat * 4.0;

        // Offside avoidance
        if !self.would_be_offside(ctx, position) {
            score += 15.0;
        }

        // Channel positioning bonus
        if self.is_in_dangerous_channel(ctx, position) {
            score += 10.0;
        }

        // Behind defensive line bonus
        if self.is_behind_defensive_line(ctx, position) {
            score += 20.0;
        }

        // IMPROVED: Ball holder awareness - CRITICAL for receiving passes
        if let Some(ball_holder) = self.get_ball_holder(ctx) {
            let holder_distance = (position - ball_holder.position).magnitude();

            // MAJOR BONUS for optimal passing distance (20-45m)
            if holder_distance >= OPTIMAL_PASSING_DISTANCE_MIN
                && holder_distance <= OPTIMAL_PASSING_DISTANCE_MAX {
                score += 25.0; // STRONG incentive to be in passing range
            } else if holder_distance < OPTIMAL_PASSING_DISTANCE_MIN {
                // Penalty for being too close (harder to receive)
                score -= (OPTIMAL_PASSING_DISTANCE_MIN - holder_distance) * 0.5;
            } else if holder_distance > OPTIMAL_PASSING_DISTANCE_MAX {
                // Progressive penalty for being too far
                score -= (holder_distance - OPTIMAL_PASSING_DISTANCE_MAX) * 0.8;
            }

            // MAJOR BONUS for clear passing lane from ball holder
            if self.has_clear_passing_lane(ball_holder.position, position, ctx) {
                score += PASSING_LANE_IMPORTANCE;
            } else {
                // Penalty for blocked passing lane
                score -= 10.0;
            }

            // BONUS for good receiving angle (diagonal/forward from holder)
            let angle_quality = self.calculate_receiving_angle_quality(ctx, ball_holder.position, position);
            score += angle_quality * 8.0; // Up to 8 bonus points for perfect angle

            // BONUS if holder is under pressure (need to offer option quickly)
            if self.is_ball_holder_under_pressure(ctx, ball_holder.id) {
                score += 12.0;
            }
        } else {
            // Fallback: distance from ball (when no clear holder)
            let ball_distance = (position - ctx.tick_context.positions.ball.position).magnitude();
            if ball_distance > MAX_DISTANCE_FROM_BALL {
                score -= (ball_distance - MAX_DISTANCE_FROM_BALL) * 0.5;
            }
        }

        score
    }

    /// Calculate dynamic avoidance vector
    fn calculate_dynamic_avoidance(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let mut avoidance = Vector3::zeros();
        let player_pos = ctx.player.position;

        // Strong avoidance of defenders
        for opponent in ctx.players().opponents().all() {
            if opponent.tactical_positions.is_defender() {
                let distance = (opponent.position - player_pos).magnitude();
                if distance < 25.0 && distance > 0.1 {
                    let direction = (player_pos - opponent.position).normalize();
                    let weight = 1.0 - (distance / 25.0);

                    // Predict defender movement
                    let future_pos = opponent.position + opponent.velocity(ctx) * 0.3;
                    let future_direction = (player_pos - future_pos).normalize();

                    avoidance += (direction + future_direction * 0.5) * weight * 20.0;
                }
            }
        }

        // Moderate avoidance of midfielders
        for opponent in ctx.players().opponents().all() {
            if opponent.tactical_positions.is_midfielder() {
                let distance = (opponent.position - player_pos).magnitude();
                if distance < 15.0 && distance > 0.1 {
                    let direction = (player_pos - opponent.position).normalize();
                    let weight = 1.0 - (distance / 15.0);
                    avoidance += direction * weight * 10.0;
                }
            }
        }

        // Light avoidance of teammates to maintain spacing
        for teammate in ctx.players().teammates().all() {
            if teammate.id == ctx.player.id || !teammate.tactical_positions.is_forward() {
                continue;
            }

            let distance = (teammate.position - player_pos).magnitude();
            if distance < 20.0 && distance > 0.1 {
                let direction = (player_pos - teammate.position).normalize();
                let weight = 1.0 - (distance / 20.0);
                avoidance += direction * weight * 5.0;
            }
        }

        avoidance
    }

    /// Get intelligent movement pattern for forward - IMPROVED to prioritize being a passing option
    fn get_intelligent_movement_pattern(&self, ctx: &StateProcessingContext) -> ForwardMovementPattern {
        let congestion = self.calculate_local_congestion(ctx);
        let defensive_line_height = self.get_defensive_line_height(ctx);
        let ball_in_wide_area = self.is_ball_in_wide_area(ctx);
        let time_factor = ctx.in_state_time % 100;

        // Analyze defender positioning
        let defenders_compact = self.are_defenders_compact(ctx);
        let has_space_behind = self.has_space_behind_defense(ctx);

        // NEW: Check ball holder situation
        let ball_holder_under_pressure = if let Some(holder) = self.get_ball_holder(ctx) {
            self.is_ball_holder_under_pressure(ctx, holder.id)
        } else {
            false
        };

        // NEW: Check if we're in good passing range
        let in_passing_range = if let Some(holder) = self.get_ball_holder(ctx) {
            let distance = (ctx.player.position - holder.position).magnitude();
            distance >= OPTIMAL_PASSING_DISTANCE_MIN && distance <= OPTIMAL_PASSING_DISTANCE_MAX
        } else {
            false
        };

        // PRIORITIZE: If ball holder under pressure, offer immediate support
        if ball_holder_under_pressure {
            if in_passing_range {
                // Already in range - maintain position with diagonal movement
                return ForwardMovementPattern::DiagonalRun;
            } else {
                // Not in range - check to feet immediately
                return ForwardMovementPattern::CheckToFeet;
            }
        }

        // PRIORITIZE: Exploit space behind defense if available
        if has_space_behind && !self.would_be_offside_now(ctx) {
            ForwardMovementPattern::ChannelRun
        } else if defenders_compact && ball_in_wide_area {
            ForwardMovementPattern::DiagonalRun
        } else if congestion > CONGESTION_THRESHOLD && time_factor < 30 {
            ForwardMovementPattern::DriftWide
        } else if defensive_line_height > 0.6 && ctx.player().skills(ctx.player.id).mental.off_the_ball > 14.0 {
            ForwardMovementPattern::DirectRun
        } else if !in_passing_range && ctx.ball().distance() < 60.0 {
            // IMPROVED: CheckToFeet more often when not in optimal passing range
            ForwardMovementPattern::CheckToFeet
        } else if self.detect_defensive_shift(ctx) {
            ForwardMovementPattern::OppositeMovement
        } else {
            ForwardMovementPattern::DiagonalRun
        }
    }

    /// Find channel between defenders
    fn find_defensive_channel(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let defenders: Vec<MatchPlayerLite> = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .collect();

        if defenders.len() < 2 {
            return self.get_forward_search_center(ctx);
        }

        let mut best_channel = ctx.player.position;
        let mut max_gap = 0.0;

        // Find gaps between defenders
        for i in 0..defenders.len() {
            for j in i + 1..defenders.len() {
                let def1 = &defenders[i];
                let def2 = &defenders[j];

                let gap_center = (def1.position + def2.position) * 0.5;
                let gap_width = (def1.position - def2.position).magnitude();

                if gap_width > max_gap && gap_width > 15.0 {
                    // Check if channel is progressive
                    if self.is_progressive_position(ctx, gap_center) {
                        max_gap = gap_width;
                        best_channel = gap_center;
                    }
                }
            }
        }

        // Move slightly ahead of the channel
        let attacking_direction = self.get_attacking_direction(ctx);
        best_channel + attacking_direction * 10.0
    }

    /// Calculate diagonal run target
    fn calculate_diagonal_run_target(&self, ctx: &StateProcessingContext, base_target: Vector3<f32>) -> Vector3<f32> {
        let player_pos = ctx.player.position;
        let field_height = ctx.context.field_size.height as f32;

        // Determine diagonal direction based on current position
        let diagonal_offset = if player_pos.y < field_height / 2.0 {
            Vector3::new(0.0, 20.0, 0.0) // Diagonal toward center from left
        } else {
            Vector3::new(0.0, -20.0, 0.0) // Diagonal toward center from right
        };

        let attacking_direction = self.get_attacking_direction(ctx);
        base_target + diagonal_offset + attacking_direction * 15.0
    }

    /// Calculate wide position to create central space
    fn calculate_wide_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_height = ctx.context.field_size.height as f32;
        let ball_pos = ctx.tick_context.positions.ball.position;

        // Determine which wing to drift to
        let target_y = if ball_pos.y < field_height / 2.0 {
            field_height * 0.85 // Drift to right wing
        } else {
            field_height * 0.15 // Drift to left wing
        };

        let attacking_direction = self.get_attacking_direction(ctx);
        let forward_position = ball_pos.x + attacking_direction.x * 30.0;

        Vector3::new(forward_position, target_y, 0.0)
    }

    /// Calculate check position (coming short) - IMPROVED for better receiving angles
    fn calculate_check_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_pos = ctx.player.position;

        // Prioritize positioning relative to ball holder, not just ball
        if let Some(ball_holder) = self.get_ball_holder(ctx) {
            let holder_pos = ball_holder.position;
            let to_player = (player_pos - holder_pos).normalize();
            let attacking_direction = self.get_attacking_direction(ctx);

            // Calculate optimal check distance (within passing range)
            let current_distance = (player_pos - holder_pos).magnitude();
            let target_distance = OPTIMAL_PASSING_DISTANCE_MIN + 10.0; // 30m

            // Create diagonal angle for easier passing
            let lateral_direction = Vector3::new(-to_player.y, to_player.x, 0.0);

            // Blend forward movement with lateral movement for diagonal angle
            let ideal_direction = if current_distance > target_distance {
                // Too far - come closer, but at an angle
                (-to_player * 0.6 + lateral_direction * 0.4 + attacking_direction * 0.3).normalize()
            } else {
                // Right distance - maintain angle and move slightly forward
                (lateral_direction * 0.5 + attacking_direction * 0.5).normalize()
            };

            let target_position = player_pos + ideal_direction * 15.0;

            // Ensure we're not moving into congested area
            if self.calculate_position_congestion(ctx, target_position) < 4.0 {
                return target_position;
            }
        }

        // Fallback: original logic if no ball holder found
        let ball_pos = ctx.tick_context.positions.ball.position;
        let to_ball = (ball_pos - player_pos).normalize();
        let check_distance = 20.0;

        let lateral_offset = if player_pos.y < ctx.context.field_size.height as f32 / 2.0 {
            Vector3::new(0.0, -5.0, 0.0)
        } else {
            Vector3::new(0.0, 5.0, 0.0)
        };

        player_pos + to_ball * check_distance + lateral_offset
    }

    /// Calculate opposite movement to defensive shift
    fn calculate_opposite_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let defensive_shift = self.calculate_defensive_shift_vector(ctx);
        let player_pos = ctx.player.position;

        // Move opposite to defensive shift
        let opposite_direction = -defensive_shift.normalize();
        let movement_distance = 25.0;

        let target = player_pos + opposite_direction * movement_distance;

        // Ensure progressive movement
        let attacking_direction = self.get_attacking_direction(ctx);
        target + attacking_direction * 10.0
    }

    /// Calculate position congestion
    fn calculate_position_congestion(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let mut congestion = 0.0;

        for opponent in ctx.players().opponents().all() {
            let distance = (opponent.position - position).magnitude();
            if distance < 30.0 {
                congestion += (30.0 - distance) / 30.0;
            }
        }

        congestion
    }

    /// Calculate goal threat from position
    fn calculate_goal_threat(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let goal_pos = ctx.player().opponent_goal_position();
        let distance_to_goal = (position - goal_pos).magnitude();

        // Ideal shooting distance is 15-25 meters
        if distance_to_goal < 15.0 {
            8.0
        } else if distance_to_goal < 25.0 {
            10.0
        } else if distance_to_goal < 35.0 {
            6.0
        } else {
            (100.0 - distance_to_goal).max(0.0) / 20.0
        }
    }

    // Helper methods
    fn get_forward_search_center(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let attacking_direction = self.get_attacking_direction(ctx);

        // Search ahead of ball position
        ball_pos + attacking_direction * 30.0
    }

    fn get_attacking_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        match ctx.player.side {
            Some(PlayerSide::Left) => Vector3::new(1.0, 0.0, 0.0),
            Some(PlayerSide::Right) => Vector3::new(-1.0, 0.0, 0.0),
            None => Vector3::new(1.0, 0.0, 0.0),
        }
    }

    fn would_be_offside(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let attacking_direction = self.get_attacking_direction(ctx);
        let is_attacking_left = attacking_direction.x > 0.0;

        // Find last defender position
        let last_defender_x = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position.x)
            .fold(if is_attacking_left { f32::MIN } else { f32::MAX },
                  |acc, x| if is_attacking_left { acc.max(x) } else { acc.min(x) });

        if is_attacking_left {
            position.x > last_defender_x + 2.0
        } else {
            position.x < last_defender_x - 2.0
        }
    }

    /// Get the defensive line X position
    fn get_defensive_line_position(&self, ctx: &StateProcessingContext) -> f32 {
        let attacking_direction = self.get_attacking_direction(ctx);
        let is_attacking_left = attacking_direction.x > 0.0;

        ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position.x)
            .fold(if is_attacking_left { f32::MIN } else { f32::MAX },
                  |acc, x| if is_attacking_left { acc.max(x) } else { acc.min(x) })
    }

    fn would_be_offside_now(&self, ctx: &StateProcessingContext) -> bool {
        self.would_be_offside(ctx, ctx.player.position)
    }

    fn is_in_dangerous_channel(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let channel_width = field_height / 5.0;

        // Central channels are most dangerous
        let center = field_height / 2.0;
        let distance_from_center = (position.y - center).abs();

        distance_from_center < channel_width * 1.5
    }

    fn is_behind_defensive_line(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let attacking_direction = self.get_attacking_direction(ctx);
        let is_attacking_left = attacking_direction.x > 0.0;

        let avg_defender_x = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position.x)
            .sum::<f32>() / 4.0; // Assume 4 defenders

        if is_attacking_left {
            position.x > avg_defender_x
        } else {
            position.x < avg_defender_x
        }
    }

    fn calculate_local_congestion(&self, ctx: &StateProcessingContext) -> f32 {
        self.calculate_position_congestion(ctx, ctx.player.position)
    }

    fn get_defensive_line_height(&self, ctx: &StateProcessingContext) -> f32 {
        let field_width = ctx.context.field_size.width as f32;
        let defenders: Vec<f32> = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position.x)
            .collect();

        if defenders.is_empty() {
            return 0.5;
        }

        let avg_x = defenders.iter().sum::<f32>() / defenders.len() as f32;
        avg_x / field_width
    }

    fn is_ball_in_wide_area(&self, ctx: &StateProcessingContext) -> bool {
        let ball_y = ctx.tick_context.positions.ball.position.y;
        let field_height = ctx.context.field_size.height as f32;

        ball_y < field_height * 0.25 || ball_y > field_height * 0.75
    }

    fn are_defenders_compact(&self, ctx: &StateProcessingContext) -> bool {
        let defenders: Vec<Vector3<f32>> = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position)
            .collect();

        if defenders.len() < 2 {
            return false;
        }

        let max_distance = defenders.iter()
            .flat_map(|d1| defenders.iter().map(move |d2| (d1 - d2).magnitude()))
            .fold(0.0_f32, f32::max);

        max_distance < 40.0
    }

    fn has_space_behind_defense(&self, ctx: &StateProcessingContext) -> bool {
        let defensive_line = self.get_defensive_line_height(ctx);
        let field_width = ctx.context.field_size.width as f32;
        let attacking_direction = self.get_attacking_direction(ctx);

        if attacking_direction.x > 0.0 {
            defensive_line < 0.7 && (field_width - defensive_line * field_width) > 30.0
        } else {
            defensive_line > 0.3 && (defensive_line * field_width) > 30.0
        }
    }

    fn detect_defensive_shift(&self, ctx: &StateProcessingContext) -> bool {
        // Simplified detection - check if defenders are shifting to one side
        let defenders: Vec<f32> = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position.y)
            .collect();

        if defenders.is_empty() {
            return false;
        }

        let avg_y = defenders.iter().sum::<f32>() / defenders.len() as f32;
        let field_height = ctx.context.field_size.height as f32;

        (avg_y - field_height / 2.0).abs() > field_height * 0.15
    }

    fn calculate_defensive_shift_vector(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let defenders: Vec<Vector3<f32>> = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.position)
            .collect();

        if defenders.len() < 2 {
            return Vector3::zeros();
        }

        // Calculate average movement direction
        let avg_velocity: Vector3<f32> = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| p.velocity(ctx))
            .sum::<Vector3<f32>>() / defenders.len() as f32;

        avg_velocity
    }

    fn has_clear_passing_lane(&self, from: Vector3<f32>, to: Vector3<f32>, ctx: &StateProcessingContext) -> bool {
        let direction = (to - from).normalize();
        let distance = (to - from).magnitude();

        !ctx.players().opponents().all().any(|opp| {
            let to_opp = opp.position - from;
            let projection = to_opp.dot(&direction);

            if projection <= 0.0 || projection >= distance {
                return false;
            }

            let projected_point = from + direction * projection;
            let perp_distance = (opp.position - projected_point).magnitude();

            perp_distance < 4.0
        })
    }

    fn is_progressive_position(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let goal_pos = ctx.player().opponent_goal_position();
        let current_distance = (ctx.player.position - goal_pos).magnitude();
        let new_distance = (position - goal_pos).magnitude();

        new_distance < current_distance
    }

    fn apply_forward_tactical_adjustment(&self, ctx: &StateProcessingContext, mut position: Vector3<f32>) -> Vector3<f32> {
        // Get player's team tactics
        let player_tactics = match ctx.player.side {
            Some(PlayerSide::Left) => &ctx.context.tactics.left,
            Some(PlayerSide::Right) => &ctx.context.tactics.right,
            None => return position,
        };

        // Adjust based on tactical style
        match player_tactics.tactical_style() {
            crate::TacticalStyle::Attacking => {
                // Push higher up the pitch
                let attacking_direction = self.get_attacking_direction(ctx);
                position += attacking_direction * 10.0;
            }
            crate::TacticalStyle::Counterattack => {
                // Stay ready to exploit space
                if self.has_space_behind_defense(ctx) {
                    let attacking_direction = self.get_attacking_direction(ctx);
                    position += attacking_direction * 15.0;
                }
            }
            crate::TacticalStyle::WidePlay | crate::TacticalStyle::WingPlay => {
                // Push wider
                let field_height = ctx.context.field_size.height as f32;
                if position.y < field_height / 2.0 {
                    position.y = (position.y - 10.0).max(10.0);
                } else {
                    position.y = (position.y + 10.0).min(field_height - 10.0);
                }
            }
            crate::TacticalStyle::Possession => {
                // Come shorter to help build play
                let ball_pos = ctx.tick_context.positions.ball.position;
                let to_ball = (ball_pos - position).normalize();
                position += to_ball * 5.0;
            }
            _ => {}
        }

        // Ensure within bounds
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        position.x = position.x.clamp(10.0, field_width - 10.0);
        position.y = position.y.clamp(10.0, field_height - 10.0);

        position
    }

    // Keep existing helper methods for compatibility
    fn has_created_good_space(&self, ctx: &StateProcessingContext) -> bool {
        let space_created = !ctx.players().opponents().exists(20.0);
        let in_support_position = self.is_in_good_support_position(ctx);
        let has_clear_lane = self.has_clear_passing_lane_from_ball_holder(ctx);
        let minimum_time_in_state = 30;
        let reasonable_distance = ctx.ball().distance() < MAX_DISTANCE_FROM_BALL;

        space_created && in_support_position && has_clear_lane
            && ctx.in_state_time > minimum_time_in_state && reasonable_distance
    }

    fn should_make_forward_run(&self, ctx: &StateProcessingContext) -> bool {
        if !ctx.team().is_control_ball() {
            return false;
        }

        let ball_holder_can_pass = self.ball_holder_can_make_forward_pass(ctx);
        let space_ahead = self.has_space_ahead_for_run(ctx);
        let not_offside = !self.would_be_offside_now(ctx);
        let in_good_phase = self.is_in_good_attacking_phase(ctx);
        let not_too_far = ctx.ball().distance() < MAX_DISTANCE_FROM_BALL;

        ball_holder_can_pass && space_ahead && not_offside && in_good_phase && not_too_far
    }

    fn is_in_good_support_position(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        ball_distance >= MIN_DISTANCE_FROM_BALL && ball_distance <= MAX_DISTANCE_FROM_BALL
    }

    fn has_clear_passing_lane_from_ball_holder(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(holder) = self.get_ball_holder(ctx) {
            self.has_clear_passing_lane(holder.position, ctx.player.position, ctx)
        } else {
            true
        }
    }

    fn get_ball_holder(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        ctx.players()
            .teammates()
            .all()
            .find(|t| ctx.ball().owner_id() == Some(t.id))
    }

    fn ball_holder_can_make_forward_pass(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(holder) = self.get_ball_holder(ctx) {
            // Check if holder is under pressure
            let holder_under_pressure = ctx.players().opponents().all()
                .any(|opp| (opp.position - holder.position).magnitude() < 8.0);

            !holder_under_pressure
        } else {
            false
        }
    }

    fn has_space_ahead_for_run(&self, ctx: &StateProcessingContext) -> bool {
        let player_position = ctx.player.position;
        let attacking_direction = self.get_attacking_direction(ctx);
        let check_position = player_position + attacking_direction * 40.0;

        let opponents_in_space = ctx.players().opponents().all()
            .filter(|opp| (opp.position - check_position).magnitude() < 15.0)
            .count();

        opponents_in_space < 2
    }

    fn is_in_good_attacking_phase(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;

        ball_distance_to_goal < field_width * 0.7
    }

    /// Calculate quality of receiving angle from ball holder
    /// Returns 0.0-1.0 where 1.0 is ideal diagonal/forward angle
    fn calculate_receiving_angle_quality(&self, ctx: &StateProcessingContext, holder_pos: Vector3<f32>, target_pos: Vector3<f32>) -> f32 {
        let to_target = (target_pos - holder_pos).normalize();
        let attacking_direction = self.get_attacking_direction(ctx);

        // Calculate forward component (how much position is ahead of holder)
        let forward_alignment = to_target.dot(&attacking_direction);

        // Calculate lateral component (diagonal passing angle)
        let lateral_component = to_target.y.abs();

        // Ideal angle is diagonal-forward (not straight ahead, not directly sideways)
        // Best: 45-degree diagonal forward (forward_alignment ~0.7, lateral ~0.7)
        let angle_quality = if forward_alignment > 0.3 {
            // Forward or diagonal-forward
            if lateral_component > 0.3 && lateral_component < 0.8 {
                // Good diagonal angle
                1.0
            } else if lateral_component <= 0.3 {
                // Straight ahead - decent but not ideal
                0.7
            } else {
                // Too wide
                0.4
            }
        } else if forward_alignment > -0.2 {
            // Lateral pass - acceptable
            0.5
        } else {
            // Backwards - poor receiving angle
            0.1
        };

        angle_quality
    }

    /// Check if ball holder is under defensive pressure
    fn is_ball_holder_under_pressure(&self, ctx: &StateProcessingContext, holder_id: u32) -> bool {
        if let Some(holder) = ctx.players().teammates().all().find(|p| p.id == holder_id) {
            // Check for opponents within pressing distance
            let pressing_opponents = ctx.players()
                .opponents()
                .all()
                .filter(|opp| {
                    let distance = (opp.position - holder.position).magnitude();
                    distance < 10.0
                })
                .count();

            pressing_opponents >= 1
        } else {
            false
        }
    }
}