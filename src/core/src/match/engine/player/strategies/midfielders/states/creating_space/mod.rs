use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_DISTANCE_FROM_BALL: f32 = 120.0;
const MIN_DISTANCE_FROM_BALL: f32 = 25.0;
const SPACE_CREATION_RADIUS: f32 = 20.0;
#[allow(dead_code)]
const HALF_SPACE_WIDTH: f32 = 15.0;

#[derive(Default)]
pub struct MidfielderCreatingSpaceState {}

impl StateProcessingHandler for MidfielderCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has the ball
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Check if team lost possession
        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        // If ball is coming toward player and close, prepare to receive
        if ctx.ball().distance() < 80.0 && ctx.ball().is_towards_player_with_angle(0.85) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Check if created sufficient space and ready to receive
        if self.has_created_quality_space(ctx) && self.is_ready_to_receive(ctx) {
            // Signal availability by slight movement
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Make attacking run if opportunity arises
        if self.should_make_attacking_run(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // Don't stay in this state too long
        if ctx.in_state_time > 100 && !self.is_space_creation_valuable(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Find the optimal free zone on the opposite side
        let target_position = self.find_opposite_side_free_zone(ctx);

        // Add dynamic avoidance of congested areas during movement
        let avoidance_vector = self.calculate_congestion_avoidance(ctx);

        // Vary movement pattern to confuse markers and exploit space
        let movement_pattern = self.get_intelligent_movement_pattern(ctx);

        match movement_pattern {
            MovementPattern::Direct => {
                // Direct run to free space with congestion avoidance
                let base_velocity = SteeringBehavior::Arrive {
                    target: target_position,
                    slowing_distance: 15.0,
                }
                    .calculate(ctx.player)
                    .velocity;

                Some(base_velocity + avoidance_vector + ctx.player().separation_velocity())
            }
            MovementPattern::Curved => {
                // Curved run to lose markers and find space
                let curved_target = self.add_intelligent_curve(ctx, target_position);
                let base_velocity = SteeringBehavior::Arrive {
                    target: curved_target,
                    slowing_distance: 20.0,
                }
                    .calculate(ctx.player)
                    .velocity;

                Some(base_velocity + avoidance_vector * 0.8)
            }
            MovementPattern::CheckToReceive => {
                // Quick check toward ball then sprint to space
                if ctx.in_state_time % 30 < 10 {
                    // Check toward ball
                    let check_position = self.calculate_check_position(ctx);
                    Some(
                        SteeringBehavior::Seek {
                            target: check_position,
                        }
                            .calculate(ctx.player)
                            .velocity
                    )
                } else {
                    // Sprint to free space
                    Some(
                        SteeringBehavior::Pursuit {
                            target: target_position,
                            target_velocity: Vector3::zeros(), // Static target position
                        }
                            .calculate(ctx.player)
                            .velocity + avoidance_vector
                    )
                }
            }
            MovementPattern::OppositeRun => {
                // New pattern: Run opposite to ball movement to find space
                let opposite_target = self.calculate_opposite_run_target(ctx);
                Some(
                    SteeringBehavior::Arrive {
                        target: opposite_target,
                        slowing_distance: 10.0,
                    }
                        .calculate(ctx.player)
                        .velocity + avoidance_vector * 1.2
                )
            }
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Creating space is moderate intensity - tactical movement
        MidfielderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}

impl MidfielderCreatingSpaceState {
    /// Find free zone on the opposite side of play
    fn find_opposite_side_free_zone(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine which side the ball is on
        let ball_on_left = ball_pos.y < field_height / 2.0;

        // Calculate opposite side base position
        let opposite_y = if ball_on_left {
            field_height * 0.75 // Right side
        } else {
            field_height * 0.25 // Left side
        };

        // Find the freest zone on that side
        let mut best_position = Vector3::new(
            ball_pos.x,
            opposite_y,
            0.0,
        );

        let mut min_congestion = f32::MAX;

        // Scan a grid on the opposite side
        let scan_width = 80.0;
        let scan_depth = 100.0;
        let grid_step = 15.0;

        for x_offset in (-scan_depth as i32..=scan_depth as i32).step_by(grid_step as usize) {
            for y_offset in (-scan_width as i32..=scan_width as i32).step_by(grid_step as usize) {
                let test_pos = Vector3::new(
                    (ball_pos.x + x_offset as f32).clamp(20.0, field_width - 20.0),
                    (opposite_y + y_offset as f32).clamp(20.0, field_height - 20.0),
                    0.0,
                );

                // Calculate dynamic congestion considering player velocities
                let congestion = self.calculate_dynamic_congestion(ctx, test_pos);

                // Prefer progressive positions
                let progression_bonus = self.calculate_progression_value(ctx, test_pos);

                // Prefer positions with good passing angles
                let passing_angle_bonus = self.calculate_passing_angle_value(ctx, test_pos);

                let total_score = congestion - progression_bonus - passing_angle_bonus;

                if total_score < min_congestion {
                    min_congestion = total_score;
                    best_position = test_pos;
                }
            }
        }

        // Apply tactical adjustments based on formation
        self.apply_tactical_position_adjustment(ctx, best_position)
    }

    /// Calculate congestion avoidance vector
    fn calculate_congestion_avoidance(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let mut avoidance = Vector3::zeros();
        let player_pos = ctx.player.position;

        // Consider all nearby players (both teams)
        let avoidance_radius = 30.0;
        let mut total_weight = 0.0;

        for opponent in ctx.players().opponents().all() {
            let distance = (opponent.position - player_pos).magnitude();
            if distance < avoidance_radius && distance > 0.1 {
                // Calculate repulsion force
                let direction = (player_pos - opponent.position).normalize();
                let weight = 1.0 - (distance / avoidance_radius);
                avoidance += direction * weight * 15.0;
                total_weight += weight;
            }
        }

        // Also avoid teammates slightly to spread out
        for teammate in ctx.players().teammates().all() {
            if teammate.id == ctx.player.id {
                continue;
            }

            let distance = (teammate.position - player_pos).magnitude();
            if distance < avoidance_radius * 0.7 && distance > 0.1 {
                let direction = (player_pos - teammate.position).normalize();
                let weight = 1.0 - (distance / (avoidance_radius * 0.7));
                avoidance += direction * weight * 8.0;
                total_weight += weight;
            }
        }

        if total_weight > 0.0 {
            avoidance / total_weight
        } else {
            avoidance
        }
    }

    /// Calculate dynamic congestion considering player movements
    fn calculate_dynamic_congestion(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let mut congestion = 0.0;

        // Check opponents
        for opponent in ctx.players().opponents().all() {
            let distance = (opponent.position - position).magnitude();

            // Consider opponent velocity - where they're heading
            let future_position = opponent.position + opponent.velocity(ctx) * 0.5;
            let future_distance = (future_position - position).magnitude();

            if distance < 40.0 {
                congestion += 40.0 / (distance + 1.0);
            }

            if future_distance < 30.0 {
                congestion += 20.0 / (future_distance + 1.0);
            }
        }

        // Check teammates (less weight)
        for teammate in ctx.players().teammates().all() {
            if teammate.id == ctx.player.id {
                continue;
            }

            let distance = (teammate.position - position).magnitude();
            if distance < 25.0 {
                congestion += 10.0 / (distance + 1.0);
            }
        }

        congestion
    }

    /// Get intelligent movement pattern based on game state
    fn get_intelligent_movement_pattern(&self, ctx: &StateProcessingContext) -> MovementPattern {
        // Analyze current situation
        let congestion_level = self.calculate_local_congestion(ctx);
        let has_marker = self.has_close_marker(ctx);
        let ball_moving_away = ctx.ball().velocity().magnitude() > 5.0;

        if has_marker && congestion_level > 5.0 {
            // Need to lose marker in congested area
            MovementPattern::Curved
        } else if ball_moving_away && ctx.ball().distance() > 50.0 {
            // Ball is moving away, make opposite run
            MovementPattern::OppositeRun
        } else if ctx.in_state_time % 60 < 15 {
            // Periodically check back
            MovementPattern::CheckToReceive
        } else {
            // Direct run to space
            MovementPattern::Direct
        }
    }

    /// Calculate intelligent curve to lose markers
    fn add_intelligent_curve(&self, ctx: &StateProcessingContext, target: Vector3<f32>) -> Vector3<f32> {
        let player_pos = ctx.player.position;

        // Find nearest opponent
        if let Some(nearest_opponent) = ctx.players().opponents().all()
            .min_by(|a, b| {
                let dist_a = (a.position - player_pos).magnitude();
                let dist_b = (b.position - player_pos).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
            }) {
            // Curve away from opponent
            let to_target = (target - player_pos).normalize();
            let to_opponent = (nearest_opponent.position - player_pos).normalize();

            // Create perpendicular vector away from opponent
            let perpendicular = Vector3::new(-to_target.y, to_target.x, 0.0);
            let away_from_opponent = if perpendicular.dot(&to_opponent) > 0.0 {
                -perpendicular
            } else {
                perpendicular
            };

            // Create curved waypoint
            let midpoint = (player_pos + target) * 0.5;
            midpoint + away_from_opponent * 20.0
        } else {
            // Default curve
            self.add_curve_to_path(ctx, target)
        }
    }

    /// Calculate target for opposite run
    fn calculate_opposite_run_target(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Run opposite to ball movement direction
        let opposite_direction = -ball_velocity.normalize();

        // Calculate target based on opposite direction
        let base_distance = 40.0;
        let mut target = ball_pos + opposite_direction * base_distance;

        // Shift to opposite side laterally as well
        let lateral_shift = if ball_pos.y < field_height / 2.0 {
            Vector3::new(0.0, 30.0, 0.0) // Shift right
        } else {
            Vector3::new(0.0, -30.0, 0.0) // Shift left
        };

        target += lateral_shift;

        // Ensure within field bounds
        target.x = target.x.clamp(15.0, field_width - 15.0);
        target.y = target.y.clamp(15.0, field_height - 15.0);

        target
    }

    /// Calculate progression value for a position
    fn calculate_progression_value(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let goal_pos = ctx.player().opponent_goal_position();
        let current_to_goal = (ctx.player.position - goal_pos).magnitude();
        let test_to_goal = (position - goal_pos).magnitude();

        if test_to_goal < current_to_goal {
            (current_to_goal - test_to_goal) * 0.5
        } else {
            0.0
        }
    }

    /// Calculate passing angle value for a position
    fn calculate_passing_angle_value(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        if let Some(ball_holder) = self.find_ball_holder(ctx) {
            // Check angle to ball holder
            let to_ball_holder = (ball_holder.position - position).normalize();

            // Count how many opponents are in the passing lane
            let opponents_in_lane = ctx.players().opponents().all()
                .filter(|opp| {
                    let to_opp = opp.position - position;
                    let projection = to_opp.dot(&to_ball_holder);

                    if projection <= 0.0 || projection >= (ball_holder.position - position).magnitude() {
                        return false;
                    }

                    let projected_point = position + to_ball_holder * projection;
                    let perp_dist = (opp.position - projected_point).magnitude();

                    perp_dist < 5.0
                })
                .count();

            if opponents_in_lane == 0 {
                15.0 // Clear passing lane bonus
            } else {
                0.0
            }
        } else {
            5.0 // Default value
        }
    }

    /// Apply tactical adjustments based on formation
    fn apply_tactical_position_adjustment(&self, ctx: &StateProcessingContext, mut position: Vector3<f32>) -> Vector3<f32> {
        // Get the appropriate tactics based on player's side
        let player_tactics = match ctx.player.side {
            Some(PlayerSide::Left) => &ctx.context.tactics.left,
            Some(PlayerSide::Right) => &ctx.context.tactics.right,
            None => return position, // No side assigned, return unchanged
        };

        // Adjust based on tactical style
        match player_tactics.tactical_style() {
            crate::TacticalStyle::WidePlay | crate::TacticalStyle::WingPlay => {
                // Push wider in wide formations
                let field_height = ctx.context.field_size.height as f32;
                if position.y < field_height / 2.0 {
                    position.y = (position.y - 15.0).max(10.0);
                } else {
                    position.y = (position.y + 15.0).min(field_height - 10.0);
                }
            }
            crate::TacticalStyle::Possession => {
                // Stay more central for passing options
                let field_height = ctx.context.field_size.height as f32;
                let center_y = field_height / 2.0;
                position.y = position.y * 0.8 + center_y * 0.2;
            }
            _ => {}
        }

        position
    }

    /// Calculate local congestion around player
    fn calculate_local_congestion(&self, ctx: &StateProcessingContext) -> f32 {
        ctx.players().opponents().nearby(20.0).count() as f32
    }

    /// Check if player has a close marker
    fn has_close_marker(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().nearby(10.0).count() > 0
    }

    /// Make a third man run (beyond the immediate play)
    #[allow(dead_code)]
    fn make_third_man_run(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        // Identify potential passing sequence
        if let Some(ball_holder) = self.find_ball_holder(ctx) {
            // Find likely next pass recipient
            if let Some(next_receiver) = self.predict_next_pass_recipient(ctx, &ball_holder) {
                // Position beyond the next receiver
                let attacking_direction = match ctx.player.side {
                    Some(PlayerSide::Left) => 1.0,
                    Some(PlayerSide::Right) => -1.0,
                    None => 0.0,
                };

                let beyond_position = Vector3::new(
                    next_receiver.position.x + (attacking_direction * 50.0),
                    next_receiver.position.y + self.calculate_run_angle(ctx, &next_receiver),
                    0.0,
                );

                return beyond_position.clamp_to_field(field_width, field_height);
            }
        }

        // Fallback to progressive run
        self.calculate_progressive_position(ctx, field_width, field_height)
    }

    /// Create central overload
    #[allow(dead_code)]
    fn create_central_overload(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let field_center_y = field_height / 2.0;

        // Move toward center but at different depths
        let depth_offset = self.calculate_depth_variation(ctx);

        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        Vector3::new(
            ball_pos.x + (attacking_direction * depth_offset),
            field_center_y + self.calculate_central_lane_offset(ctx),
            0.0,
        ).clamp_to_field(field_width, field_height)
    }

    /// Check if has created quality space
    fn has_created_quality_space(&self, ctx: &StateProcessingContext) -> bool {
        let space_radius = SPACE_CREATION_RADIUS;

        // No opponents in immediate vicinity
        let opponents_nearby = ctx.players().opponents().all()
            .filter(|opp| (opp.position - ctx.player.position).magnitude() < space_radius)
            .count();

        // Good distance from ball
        let ball_distance = ctx.ball().distance();
        let good_distance = ball_distance >= MIN_DISTANCE_FROM_BALL &&
            ball_distance <= MAX_DISTANCE_FROM_BALL;

        // Clear passing lane
        let has_clear_lane = self.has_clear_receiving_lane(ctx);

        // Progressive position
        let is_progressive = self.is_progressive_position(ctx, ctx.player.position);

        opponents_nearby == 0 && good_distance && has_clear_lane &&
            (is_progressive || ctx.in_state_time > 40)
    }

    /// Check if ready to receive pass
    fn is_ready_to_receive(&self, ctx: &StateProcessingContext) -> bool {
        // Body orientation toward ball
        let to_ball = (ctx.tick_context.positions.ball.position - ctx.player.position).normalize();
        let player_facing = ctx.player.velocity.normalize();

        let facing_ball = if ctx.player.velocity.magnitude() > 0.1 {
            to_ball.dot(&player_facing) > 0.5
        } else {
            true // Standing still, assumed ready
        };

        // Not moving too fast
        let controlled_movement = ctx.player.velocity.magnitude() < 5.0;

        facing_ball && controlled_movement
    }

    /// Should make attacking run
    fn should_make_attacking_run(&self, ctx: &StateProcessingContext) -> bool {
        let ball_in_good_position = ctx.ball().distance_to_opponent_goal() < 300.0;
        let team_attacking = ctx.team().is_control_ball();
        let has_energy = ctx.player.player_attributes.condition_percentage() > 60;
        let good_off_ball = ctx.player.skills.mental.off_the_ball > 12.0;

        ball_in_good_position && team_attacking && has_energy && good_off_ball
    }

    /// Check if space creation is valuable
    fn is_space_creation_valuable(&self, ctx: &StateProcessingContext) -> bool {
        // Team has ball and is building attack
        ctx.team().is_control_ball() &&
            ctx.ball().distance() < 150.0 &&
            !self.too_many_players_creating_space(ctx)
    }

    /// Helper methods
    #[allow(dead_code)]
    fn get_movement_pattern(&self, ctx: &StateProcessingContext) -> MovementPattern {
        let time_mod = ctx.in_state_time % 30;

        if time_mod < 10 {
            MovementPattern::Direct
        } else if time_mod < 20 {
            MovementPattern::Curved
        } else {
            MovementPattern::CheckToReceive
        }
    }

    fn add_curve_to_path(&self, ctx: &StateProcessingContext, target: Vector3<f32>) -> Vector3<f32> {
        let player_pos = ctx.player.position;
        let midpoint = (player_pos + target) * 0.5;

        // Add perpendicular offset
        let direction = (target - player_pos).normalize();
        let perpendicular = Vector3::new(-direction.y, direction.x, 0.0);

        midpoint + perpendicular * 10.0
    }

    fn calculate_check_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let player_pos = ctx.player.position;

        // Move 10m toward ball
        let to_ball = (ball_pos - player_pos).normalize();
        player_pos + to_ball * 10.0
    }

    #[allow(dead_code)]
    fn get_ball_zone(&self, ctx: &StateProcessingContext) -> BallZone {
        let ball_x = ctx.tick_context.positions.ball.position.x;
        let field_width = ctx.context.field_size.width as f32;

        let relative_x = match ctx.player.side {
            Some(PlayerSide::Left) => ball_x / field_width,
            Some(PlayerSide::Right) => (field_width - ball_x) / field_width,
            None => 0.5,
        };

        if relative_x < 0.33 {
            BallZone::DefensiveThird
        } else if relative_x < 0.66 {
            BallZone::MiddleThird
        } else {
            BallZone::AttackingThird
        }
    }

    #[allow(dead_code)]
    fn analyze_team_shape(&self, ctx: &StateProcessingContext) -> TeamShape {
        let teammates = ctx.players().teammates().all().collect::<Vec<_>>();

        if teammates.is_empty() {
            return TeamShape::default();
        }

        // Calculate spread
        let positions: Vec<Vector3<f32>> = teammates.iter().map(|t| t.position).collect();
        let min_x = positions.iter().map(|p| p.x).fold(f32::INFINITY, f32::min);
        let max_x = positions.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max);
        let min_y = positions.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        let max_y = positions.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max);

        TeamShape {
            width: max_y - min_y,
            depth: max_x - min_x,
            compactness: teammates.len() as f32 / ((max_x - min_x) * (max_y - min_y) / 1000.0),
        }
    }

    #[allow(dead_code)]
    fn analyze_opponent_shape(&self, ctx: &StateProcessingContext) -> OpponentShape {
        let opponents = ctx.players().opponents().all().collect::<Vec<_>>();
        let defenders = opponents.iter()
            .filter(|o| o.tactical_positions.is_defender())
            .collect::<Vec<_>>();

        if defenders.is_empty() {
            return OpponentShape::default();
        }

        let def_line = defenders.iter()
            .map(|d| d.position.x)
            .sum::<f32>() / defenders.len() as f32;

        let field_width = ctx.context.field_size.width as f32;
        let high_line = match ctx.player.side {
            Some(PlayerSide::Left) => def_line > field_width * 0.6,
            Some(PlayerSide::Right) => def_line < field_width * 0.4,
            None => false,
        };

        OpponentShape {
            high_line,
            compact_central: self.is_opponent_compact_central(ctx),
            narrow: self.is_opponent_narrow(ctx),
        }
    }

    #[allow(dead_code)]
    fn is_half_space_occupied(&self, ctx: &StateProcessingContext, y_position: f32) -> bool {
        ctx.players().teammates().all()
            .any(|t| (t.position.y - y_position).abs() < HALF_SPACE_WIDTH)
    }

    #[allow(dead_code)]
    fn can_exploit_half_space(&self, ctx: &StateProcessingContext) -> bool {
        let vision = ctx.player.skills.mental.vision;
        let off_ball = ctx.player.skills.mental.off_the_ball;

        vision > 13.0 && off_ball > 12.0
    }

    #[allow(dead_code)]
    fn find_lateral_space_between_lines(
        &self,
        ctx: &StateProcessingContext,
        defenders: &[MatchPlayerLite],
        midfielders: &[MatchPlayerLite],
    ) -> f32 {
        let field_height = ctx.context.field_size.height as f32;

        // Find gaps in coverage
        let mut all_positions = defenders.iter()
            .chain(midfielders.iter())
            .map(|p| p.position.y)
            .collect::<Vec<_>>();

        all_positions.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        // Find biggest gap
        let mut max_gap = 0.0;
        let mut gap_center = field_height / 2.0;

        for window in all_positions.windows(2) {
            let gap = window[1] - window[0];
            if gap > max_gap {
                max_gap = gap;
                gap_center = (window[0] + window[1]) / 2.0;
            }
        }

        gap_center
    }

    #[allow(dead_code)]
    fn calculate_progressive_position(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        Vector3::new(
            ball_pos.x + (attacking_direction * 40.0),
            ctx.player.position.y,
            0.0,
        ).clamp_to_field(field_width, field_height)
    }

    #[allow(dead_code)]
    fn determine_overload_strategy(&self, ctx: &StateProcessingContext) -> OverloadStrategy {
        // Based on team tactics and game situation
        if ctx.team().is_loosing() {
            OverloadStrategy::CreateNumericalAdvantage
        } else {
            OverloadStrategy::BalanceWidth
        }
    }

    #[allow(dead_code)]
    fn calculate_position_congestion(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let mut congestion = 0.0;

        // Weight opponents more than teammates
        let opponents = ctx.players().opponents().all()
            .filter(|o| (o.position - position).magnitude() < 30.0)
            .count();

        let teammates = ctx.players().teammates().all()
            .filter(|t| (t.position - position).magnitude() < 20.0)
            .count();

        congestion += opponents as f32 * 2.0;
        congestion += teammates as f32 * 0.5;

        congestion
    }

    fn is_progressive_position(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let current_to_goal = ctx.ball().distance_to_opponent_goal();
        let test_to_goal = (position - ctx.player().opponent_goal_position()).magnitude();

        test_to_goal < current_to_goal
    }

    fn find_ball_holder(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                if owner.team_id == ctx.player.team_id {
                    return Some(ctx.player().get(owner_id));
                }
            }
        }
        None
    }

    #[allow(dead_code)]
    fn predict_next_pass_recipient(
        &self,
        ctx: &StateProcessingContext,
        ball_holder: &MatchPlayerLite,
    ) -> Option<MatchPlayerLite> {
        // Simple prediction based on positioning
        ctx.players().teammates().all()
            .filter(|t| t.id != ball_holder.id && t.id != ctx.player.id)
            .min_by(|a, b| {
                let dist_a = (a.position - ball_holder.position).magnitude();
                let dist_b = (b.position - ball_holder.position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
            })
    }

    #[allow(dead_code)]
    fn calculate_run_angle(&self, ctx: &StateProcessingContext, target: &MatchPlayerLite) -> f32 {
        // Angle away from target's current position
        if target.position.y < ctx.context.field_size.height as f32 / 2.0 {
            20.0
        } else {
            -20.0
        }
    }

    #[allow(dead_code)]
    fn calculate_depth_variation(&self, ctx: &StateProcessingContext) -> f32 {
        // Vary depth based on other midfielders
        let other_mids = ctx.players().teammates().all()
            .filter(|t| t.tactical_positions.is_midfielder() && t.id != ctx.player.id)
            .count();

        match other_mids {
            0 => 30.0,
            1 => 40.0,
            2 => 50.0,
            _ => 35.0,
        }
    }

    #[allow(dead_code)]
    fn calculate_central_lane_offset(&self, ctx: &StateProcessingContext) -> f32 {
        // Slight offset to create passing lanes
        if ctx.in_state_time % 40 < 20 {
            -10.0
        } else {
            10.0
        }
    }

    fn has_clear_receiving_lane(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(ball_holder) = self.find_ball_holder(ctx) {
            let to_player = (ctx.player.position - ball_holder.position).normalize();
            let distance = (ctx.player.position - ball_holder.position).magnitude();

            // Check for opponents in passing lane
            !ctx.players().opponents().all()
                .any(|opp| {
                    let to_opp = opp.position - ball_holder.position;
                    let projection = to_opp.dot(&to_player);

                    if projection <= 0.0 || projection >= distance {
                        return false;
                    }

                    let projected_point = ball_holder.position + to_player * projection;
                    let perp_dist = (opp.position - projected_point).magnitude();

                    perp_dist < 5.0
                })
        } else {
            true
        }
    }

    fn too_many_players_creating_space(&self, ctx: &StateProcessingContext) -> bool {
        // Avoid having too many players in space creation mode
        let creating_space_count = ctx.players().teammates().all()
            .filter(|t| {
                let distance = ctx.ball().distance();
                distance > MIN_DISTANCE_FROM_BALL &&
                    distance < MAX_DISTANCE_FROM_BALL &&
                    !t.has_ball(ctx)
            })
            .count();

        creating_space_count >= 3
    }

    #[allow(dead_code)]
    fn is_opponent_compact_central(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let central_opponents = ctx.players().opponents().all()
            .filter(|o| (o.position.y - field_height / 2.0).abs() < field_height * 0.3)
            .count();

        central_opponents >= 6
    }

    #[allow(dead_code)]
    fn is_opponent_narrow(&self, ctx: &StateProcessingContext) -> bool {
        let opponents = ctx.players().opponents().all().collect::<Vec<_>>();
        if opponents.is_empty() {
            return false;
        }

        let min_y = opponents.iter().map(|o| o.position.y).fold(f32::INFINITY, f32::min);
        let max_y = opponents.iter().map(|o| o.position.y).fold(f32::NEG_INFINITY, f32::max);

        (max_y - min_y) < ctx.context.field_size.height as f32 * 0.5
    }
}

// Supporting types
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum SpaceType {
    HalfSpace,
    BetweenLines,
    WideOverload,
    DeepPocket,
    ThirdManRun,
    CentralOverload,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum BallZone {
    DefensiveThird,
    MiddleThird,
    AttackingThird,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum OverloadStrategy {
    CreateNumericalAdvantage,
    BalanceWidth,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
struct TeamShape {
    width: f32,
    depth: f32,
    compactness: f32,
}

#[allow(dead_code)]
impl TeamShape {
    fn is_stretched(&self) -> bool {
        self.depth > 300.0 || self.width > 400.0
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
struct OpponentShape {
    high_line: bool,
    compact_central: bool,
    narrow: bool,
}

#[allow(dead_code)]
impl OpponentShape {
    fn has_high_line(&self) -> bool {
        self.high_line
    }

    fn is_narrow(&self) -> bool {
        self.narrow
    }
}

// Extension trait
#[allow(dead_code)]
trait VectorFieldExtensions {
    fn clamp_to_field(self, field_width: f32, field_height: f32) -> Self;
}

impl VectorFieldExtensions for Vector3<f32> {
    fn clamp_to_field(self, field_width: f32, field_height: f32) -> Self {
        Vector3::new(
            self.x.clamp(10.0, field_width - 10.0),
            self.y.clamp(10.0, field_height - 10.0),
            self.z,
        )
    }
}

// Supporting types (expanded)
#[derive(Debug, Clone, Copy)]
enum MovementPattern {
    Direct,
    Curved,
    CheckToReceive,
    OppositeRun, // New pattern for running to opposite side
}