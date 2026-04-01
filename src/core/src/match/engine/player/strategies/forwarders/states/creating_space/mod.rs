use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};

// Movement patterns for forwards
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
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
const MIN_DISTANCE_FROM_BALL: f32 = 30.0;
const OPTIMAL_PASSING_DISTANCE_MIN: f32 = 20.0;
const OPTIMAL_PASSING_DISTANCE_MAX: f32 = 70.0;
#[allow(dead_code)]
const SPACE_SCAN_RADIUS: f32 = 250.0;
#[allow(dead_code)]
const CONGESTION_THRESHOLD: f32 = 3.0;
const PASSING_LANE_IMPORTANCE: f32 = 15.0; // High weight for clear passing lanes

#[derive(Default, Clone)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has the ball
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
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
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let ball_pos = ctx.tick_context.positions.ball.position;

        let attacking_direction = match ctx.player.side {
            Some(crate::r#match::PlayerSide::Left) => 1.0,
            Some(crate::r#match::PlayerSide::Right) => -1.0,
            None => 1.0,
        };

        let goal_pos = ctx.player().opponent_goal_position();

        // Forwards must always push TOWARD the opponent goal, never drop back.
        // Target X: between the ball and the goal, biased heavily toward goal.
        let forward_x = goal_pos.x * 0.6 + ball_pos.x * 0.4;
        // Never behind the ball — always ahead
        let (raw_min, raw_max) = if attacking_direction > 0.0 {
            (ball_pos.x.max(field_width * 0.4), field_width - 30.0)
        } else {
            (30.0, ball_pos.x.min(field_width * 0.6))
        };
        // Safety: ensure min <= max when ball is near the edge
        let min_x = raw_min.min(raw_max);
        let target_x = forward_x.clamp(min_x, raw_max);

        // Find the largest gap between defenders in the attacking zone
        let mut opp_ys: Vec<f32> = ctx.players().opponents().all()
            .filter(|opp| {
                // Only consider opponents in the zone between ball and goal
                let opp_x_ok = if attacking_direction > 0.0 {
                    opp.position.x > ball_pos.x - 20.0
                } else {
                    opp.position.x < ball_pos.x + 20.0
                };
                opp_x_ok
            })
            .map(|opp| opp.position.y)
            .collect();

        // Add field boundaries as virtual defenders
        opp_ys.push(0.0);
        opp_ys.push(field_height);
        opp_ys.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Find the widest gap
        let mut best_gap_y = field_height / 2.0;
        let mut best_gap_width = 0.0f32;
        for i in 0..opp_ys.len() - 1 {
            let gap = opp_ys[i + 1] - opp_ys[i];
            if gap > best_gap_width {
                best_gap_width = gap;
                best_gap_y = (opp_ys[i] + opp_ys[i + 1]) / 2.0;
            }
        }

        let target_y = best_gap_y.clamp(40.0, field_height - 40.0);

        let target = Vector3::new(target_x, target_y, 0.0);
        let dist = (target - ctx.player.position).magnitude();

        if dist < 8.0 {
            return Some(Vector3::zeros());
        }

        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 20.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Creating space is moderate intensity - tactical movement
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

#[allow(dead_code)]
impl ForwardCreatingSpaceState {
    /// Find optimal free zone for a forward
    /// Find optimal free zone for a forward - optimized to search gaps between opponents
    fn find_optimal_free_zone(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // Pre-collect ALL opponent positions once for reuse in scoring
        let all_opponent_positions: Vec<Vector3<f32>> = ctx.players()
            .opponents()
            .all()
            .map(|opp| opp.position)
            .collect();

        // Collect relevant nearby opponents for gap-finding
        let opponents: Vec<Vector3<f32>> = all_opponent_positions.iter()
            .filter(|&&pos| {
                (pos - player_pos).magnitude() < SPACE_SCAN_RADIUS
            })
            .copied()
            .collect();

        // If no nearby opponents, move toward goal
        if opponents.is_empty() {
            let forward_direction = (goal_pos - player_pos).normalize();
            return self.apply_forward_tactical_adjustment(
                ctx,
                player_pos + forward_direction * 30.0,
            );
        }

        // Pre-compute values used in scoring
        let ball_holder = self.get_ball_holder(ctx);
        let attacking_direction = self.get_attacking_direction(ctx);
        let is_attacking_left = attacking_direction.x > 0.0;

        // Pre-compute defensive line for offside checks
        let last_defender_x = all_opponent_positions.iter()
            .fold(if is_attacking_left { f32::MIN } else { f32::MAX },
                  |acc, pos| if is_attacking_left { acc.max(pos.x) } else { acc.min(pos.x) });

        // Find gaps between opponents using improved multi-strategy approach
        let mut candidate_positions = Vec::with_capacity(40);

        // Strategy 1: Midpoints between adjacent opponents
        for i in 0..opponents.len() {
            for j in (i + 1)..opponents.len() {
                let midpoint = (opponents[i] + opponents[j]) * 0.5;
                let gap_width = (opponents[i] - opponents[j]).magnitude();

                if gap_width > 12.0 && gap_width < 80.0 {
                    candidate_positions.push(midpoint);
                    let to_goal = (goal_pos - midpoint).normalize();
                    candidate_positions.push(midpoint + to_goal * 10.0);
                }
            }
        }

        // Strategy 2: Positions offset from opponents
        for &opp_pos in &opponents {
            let to_goal = (goal_pos - opp_pos).normalize();
            let perpendicular = Vector3::new(-to_goal.y, to_goal.x, 0.0);
            candidate_positions.push(opp_pos + perpendicular * 25.0 + to_goal * 20.0);
            candidate_positions.push(opp_pos - perpendicular * 25.0 + to_goal * 20.0);
            candidate_positions.push(opp_pos + to_goal * 15.0);
        }

        // Strategy 3: Grid-based open space detection — wide field scan
        let forward_direction = (goal_pos - player_pos).normalize();
        for x_offset in [25.0, 50.0, 80.0, 120.0] {
            for y_offset in [-100.0, -60.0, -30.0, 0.0, 30.0, 60.0, 100.0] {
                let lateral = Vector3::new(-forward_direction.y, forward_direction.x, 0.0);
                let candidate = player_pos + forward_direction * x_offset + lateral * y_offset;
                candidate_positions.push(candidate);
            }
        }

        // Strategy 4: Wide channel positions (flanks and half-spaces)
        let ball_pos = ctx.tick_context.positions.ball.position;
        let atk_dir = self.get_attacking_direction(ctx);
        for &wing_y in &[field_height * 0.10, field_height * 0.25, field_height * 0.75, field_height * 0.90] {
            for &fwd in &[40.0, 80.0, 120.0] {
                let x = (ball_pos.x + atk_dir.x * fwd).clamp(30.0, field_width - 30.0);
                candidate_positions.push(Vector3::new(x, wing_y, 0.0));
            }
        }

        candidate_positions.push(player_pos);

        // Evaluate candidates using pre-collected data
        let mut best_position = player_pos;
        let mut best_score = f32::MIN;

        for candidate in candidate_positions {
            let clamped = Vector3::new(
                candidate.x.clamp(20.0, field_width - 20.0),
                candidate.y.clamp(20.0, field_height - 20.0),
                0.0,
            );

            let score = self.evaluate_forward_position_fast(
                ctx, clamped, &all_opponent_positions,
                &ball_holder, last_defender_x, is_attacking_left,
            );

            if score > best_score {
                best_score = score;
                best_position = clamped;
            }
        }

        self.apply_forward_tactical_adjustment(ctx, best_position)
    }

    /// Fast position evaluation using pre-collected opponent data
    fn evaluate_forward_position_fast(
        &self,
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
        all_opponents: &[Vector3<f32>],
        ball_holder: &Option<MatchPlayerLite>,
        last_defender_x: f32,
        is_attacking_left: bool,
    ) -> f32 {
        let mut score = 0.0;
        let goal_pos = ctx.player().opponent_goal_position();
        let distance_to_goal = (position - goal_pos).magnitude();

        // Space score using pre-collected opponents
        let mut congestion = 0.0f32;
        for &opp_pos in all_opponents {
            let distance = (opp_pos - position).magnitude();
            if distance < 30.0 {
                congestion += (30.0 - distance) / 30.0;
            }
        }
        score += (10.0 - congestion.min(10.0)) * 3.0;

        // Goal threat score
        let goal_threat = if distance_to_goal < 15.0 {
            8.0
        } else if distance_to_goal < 25.0 {
            10.0
        } else if distance_to_goal < 35.0 {
            6.0
        } else {
            (100.0 - distance_to_goal).max(0.0) / 20.0
        };
        score += goal_threat * 6.0;

        // Box area bonus
        if distance_to_goal < 180.0 {
            score += 30.0;
        } else if distance_to_goal < 250.0 {
            score += 20.0;
        }

        // Offside check using pre-computed defensive line
        let is_offside = if is_attacking_left {
            position.x > last_defender_x + 2.0
        } else {
            position.x < last_defender_x - 2.0
        };
        if !is_offside {
            score += 15.0;
        } else {
            score -= 50.0;
        }

        // Channel positioning
        let field_height = ctx.context.field_size.height as f32;
        let channel_width = field_height / 5.0;
        let center = field_height / 2.0;
        if (position.y - center).abs() < channel_width * 1.5 {
            score += 20.0;
            if distance_to_goal < 300.0 {
                score += 15.0;
            }
        }

        // Behind defensive line using pre-computed data
        let avg_defender_x = all_opponents.iter()
            .map(|p| p.x)
            .sum::<f32>() / all_opponents.len().max(1) as f32;
        let is_behind = if is_attacking_left {
            position.x > avg_defender_x
        } else {
            position.x < avg_defender_x
        };
        if is_behind {
            score += 30.0;
        }

        // Ball holder awareness
        if let Some(holder) = ball_holder {
            let holder_distance = (position - holder.position).magnitude();

            if holder_distance >= OPTIMAL_PASSING_DISTANCE_MIN
                && holder_distance <= OPTIMAL_PASSING_DISTANCE_MAX {
                score += 25.0;
            } else if holder_distance < OPTIMAL_PASSING_DISTANCE_MIN {
                // Strong penalty for being too close — prevents clustering
                score -= (OPTIMAL_PASSING_DISTANCE_MIN - holder_distance) * 1.5;
            } else if holder_distance > OPTIMAL_PASSING_DISTANCE_MAX {
                score -= (holder_distance - OPTIMAL_PASSING_DISTANCE_MAX) * 0.5;
            }

            // WIDTH BONUS: reward lateral distance from ball holder
            // Forwards that provide width are much more useful
            let lateral_distance = (position.y - holder.position.y).abs();
            if lateral_distance > 80.0 {
                score += 30.0; // Excellent width
            } else if lateral_distance > 50.0 {
                score += 20.0; // Good width
            } else if lateral_distance > 30.0 {
                score += 10.0; // Moderate width
            } else if lateral_distance < 15.0 {
                score -= 15.0; // Too narrow — on same channel as holder
            }

            // Clear passing lane check using pre-collected opponents
            let direction = (position - holder.position).normalize();
            let distance = (position - holder.position).magnitude();
            let lane_blocked = all_opponents.iter().any(|&opp_pos| {
                let to_opp = opp_pos - holder.position;
                let projection = to_opp.dot(&direction);
                if projection <= 0.0 || projection >= distance {
                    return false;
                }
                let projected_point = holder.position + direction * projection;
                (opp_pos - projected_point).magnitude() < 4.0
            });

            if !lane_blocked {
                score += PASSING_LANE_IMPORTANCE;
            } else {
                score -= 10.0;
            }
        }

        score
    }

    /// Evaluate a position for forward play
    #[allow(dead_code)]
    fn evaluate_forward_position(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let mut score = 0.0;
        let goal_pos = ctx.player().opponent_goal_position();
        let distance_to_goal = (position - goal_pos).magnitude();

        // Space score (inverse of congestion)
        let congestion = self.calculate_position_congestion(ctx, position);
        score += (10.0 - congestion.min(10.0)) * 3.0;

        // Goal threat score - INCREASED weight for dangerous positions
        let goal_threat = self.calculate_goal_threat(ctx, position);
        score += goal_threat * 6.0; // Increased from 4.0

        // MAJOR bonus for being in the box area (very dangerous)
        if distance_to_goal < 180.0 {
            score += 30.0;
        } else if distance_to_goal < 250.0 {
            score += 20.0;
        }

        // Offside avoidance
        if !self.would_be_offside(ctx, position) {
            score += 15.0;
        } else {
            score -= 50.0; // Heavy penalty for offside positions
        }

        // Channel positioning bonus - INCREASED for central dangerous areas
        if self.is_in_dangerous_channel(ctx, position) {
            score += 20.0; // Increased from 10.0

            // Extra bonus for central positions close to goal
            if distance_to_goal < 300.0 {
                score += 15.0;
            }
        }

        // Behind defensive line bonus - INCREASED
        if self.is_behind_defensive_line(ctx, position) {
            score += 30.0; // Increased from 20.0
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

        // Use nearby() instead of all() — avoidance only matters for close players
        for opponent in ctx.players().opponents().nearby(25.0) {
            let distance = (opponent.position - player_pos).magnitude();
            if distance > 0.1 {
                let direction = (player_pos - opponent.position).normalize();
                let weight = 1.0 - (distance / 25.0);

                if opponent.tactical_positions.is_defender() {
                    avoidance += direction * weight * 20.0;
                } else {
                    avoidance += direction * weight * 10.0;
                }
            }
        }

        // Light avoidance of nearby forward teammates
        for teammate in ctx.players().teammates().nearby(20.0) {
            if !teammate.tactical_positions.is_forward() {
                continue;
            }
            let distance = (teammate.position - player_pos).magnitude();
            if distance > 0.1 {
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
        // Single scan: collect both positions and velocities of defenders
        let mut count = 0u32;
        let mut velocity_sum = Vector3::zeros();

        for p in ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
        {
            count += 1;
            velocity_sum += p.velocity(ctx);
        }

        if count < 2 {
            return Vector3::zeros();
        }

        velocity_sum / count as f32
    }

    fn has_clear_passing_lane(&self, from: Vector3<f32>, to: Vector3<f32>, ctx: &StateProcessingContext) -> bool {
        let direction = (to - from).normalize();
        let distance = (to - from).magnitude();

        // Pre-filter: only check opponents near the player (within pass distance + margin)
        !ctx.players().opponents().nearby(distance + 10.0).any(|opp| {
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
        // Reduced strictness - forwards should transition to assisting more often
        let space_created = !ctx.players().opponents().exists(12.0); // Reduced from 20.0
        let in_support_position = self.is_in_good_support_position(ctx);
        let has_clear_lane = self.has_clear_passing_lane_from_ball_holder(ctx);
        let minimum_time_in_state = 15; // Reduced from 30
        let reasonable_distance = ctx.ball().distance() < MAX_DISTANCE_FROM_BALL;

        // More lenient check - any 3 of the 4 conditions is enough
        let conditions_met = [
            space_created,
            in_support_position,
            has_clear_lane,
            reasonable_distance,
        ].iter().filter(|&&c| c).count();

        conditions_met >= 3 && ctx.in_state_time > minimum_time_in_state
    }

    /// Check if this forward should coordinate with other forwards to create space
    fn should_coordinate_with_other_forwards(&self, ctx: &StateProcessingContext) -> bool {
        // Find other forwards on the team
        let other_forwards: Vec<_> = ctx.players()
            .teammates()
            .all()
            .filter(|t| t.tactical_positions.is_forward() && t.id != ctx.player.id)
            .collect();

        if other_forwards.is_empty() {
            return false;
        }

        // Check if forwards are too close together (poor spacing)
        for forward in &other_forwards {
            let distance = (ctx.player.position - forward.position).magnitude();
            if distance < 25.0 {
                return true; // Need to coordinate - too close
            }
        }

        // Check if ball holder is looking for a pass
        if let Some(holder) = self.get_ball_holder(ctx) {
            if self.is_ball_holder_under_pressure(ctx, holder.id) {
                return true; // Need to offer option
            }
        }

        false
    }

    /// Get coordinated position relative to other forwards
    fn get_coordinated_forward_position(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let other_forwards: Vec<_> = ctx.players()
            .teammates()
            .all()
            .filter(|t| t.tactical_positions.is_forward() && t.id != ctx.player.id)
            .collect();

        if other_forwards.is_empty() {
            return None;
        }

        let field_height = ctx.context.field_size.height as f32;
        let player_pos = ctx.player.position;

        // Calculate average position of other forwards
        let avg_forward_y: f32 = other_forwards.iter()
            .map(|f| f.position.y)
            .sum::<f32>() / other_forwards.len() as f32;

        // Position ourselves on the opposite side for better width — push to flanks
        let target_y = if avg_forward_y < field_height / 2.0 {
            // Other forwards are on the left, move to right flank
            (field_height * 0.80).min(field_height - 30.0)
        } else {
            // Other forwards are on the right, move to left flank
            (field_height * 0.20).max(30.0)
        };

        // Stay in a dangerous attacking position
        let attacking_direction = self.get_attacking_direction(ctx);
        let target_x = if let Some(holder) = self.get_ball_holder(ctx) {
            // Stay ahead of ball holder but not offside
            let defensive_line = self.get_defensive_line_position(ctx);
            let ideal_x = holder.position.x + attacking_direction.x * 40.0;

            // Clamp to just behind defensive line
            match ctx.player.side {
                Some(crate::r#match::PlayerSide::Left) => ideal_x.min(defensive_line - 3.0),
                Some(crate::r#match::PlayerSide::Right) => ideal_x.max(defensive_line + 3.0),
                None => ideal_x,
            }
        } else {
            // Default: advance toward goal
            player_pos.x + attacking_direction.x * 20.0
        };

        Some(Vector3::new(target_x, target_y, 0.0))
    }

    fn should_make_forward_run(&self, ctx: &StateProcessingContext) -> bool {
        if !ctx.team().is_control_ball() {
            return false;
        }

        let ball_holder_can_pass = self.ball_holder_can_make_forward_pass(ctx);
        let not_offside = !self.would_be_offside_now(ctx);
        let in_good_phase = self.is_in_good_attacking_phase(ctx);
        let not_too_far = ctx.ball().distance() < MAX_DISTANCE_FROM_BALL;

        if !ball_holder_can_pass || !not_offside || !in_good_phase || !not_too_far {
            return false;
        }

        // Use the SAME space check that RunningInBehind uses to decide viability.
        // This prevents oscillation: if RunningInBehind would immediately fail,
        // don't start the run.
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();
        let to_goal = (goal_pos - player_pos).normalize();

        let blockers = ctx.players().opponents().nearby(30.0)
            .filter(|opp| {
                let to_opp = (opp.position - player_pos).normalize();
                to_opp.dot(&to_goal) > 0.3
            })
            .count();

        if blockers >= 2 {
            return false;
        }
        if blockers == 1 {
            return ctx.player.skills.physical.pace > 12.0;
        }

        // Also check passing lane: runner must be ahead of the passer
        if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                if owner.team_id == ctx.player.team_id {
                    let passer_pos = ctx.tick_context.positions.players.position(owner_id);
                    let to_goal_from_passer = (goal_pos - passer_pos).normalize();
                    let to_runner = (player_pos - passer_pos).normalize();
                    if to_runner.dot(&to_goal_from_passer) <= 0.0 {
                        return false; // Behind the passer — run wouldn't be viable
                    }
                }
            }
        }

        true
    }

    fn is_in_good_support_position(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let goal_distance = ctx.ball().distance_to_opponent_goal();

        // Standard support position check
        let in_normal_range = ball_distance >= MIN_DISTANCE_FROM_BALL && ball_distance <= MAX_DISTANCE_FROM_BALL;

        // Also good if in dangerous position close to goal
        let in_dangerous_area = goal_distance < 300.0 && ball_distance < MAX_DISTANCE_FROM_BALL + 20.0;

        // Good if in passing range from ball holder
        let in_passing_range = if let Some(holder) = self.get_ball_holder(ctx) {
            let holder_distance = (ctx.player.position - holder.position).magnitude();
            holder_distance >= OPTIMAL_PASSING_DISTANCE_MIN && holder_distance <= OPTIMAL_PASSING_DISTANCE_MAX
        } else {
            false
        };

        in_normal_range || in_dangerous_area || in_passing_range
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
            let holder_under_pressure = ctx.tick_context.grid
                .opponents(holder.id, 8.0).count() > 0;

            !holder_under_pressure
        } else {
            false
        }
    }

    fn has_space_ahead_for_run(&self, ctx: &StateProcessingContext) -> bool {
        let player_position = ctx.player.position;
        let attacking_direction = self.get_attacking_direction(ctx);
        let check_position = player_position + attacking_direction * 40.0;

        // Pre-filter: only check opponents within 55 units (40 ahead + 15 radius)
        let opponents_in_space = ctx.players().opponents().nearby(55.0)
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
        // Use distance closure instead of scanning all opponents
        ctx.tick_context.grid.opponents(holder_id, 10.0).next().is_some()
    }
}