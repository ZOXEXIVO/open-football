use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior, MatchPlayerLite,
};
use nalgebra::Vector3;
use crate::r#match::midfielders::states::MidfielderState;

const MAX_DISTANCE_FROM_BALL: f32 = 120.0;
const MIN_DISTANCE_FROM_BALL: f32 = 25.0;
const OPTIMAL_SUPPORT_DISTANCE: f32 = 40.0;
const SPACE_CREATION_RADIUS: f32 = 20.0;
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
        let target_position = self.calculate_intelligent_space_position(ctx);

        // Vary movement pattern to confuse markers
        let movement_pattern = self.get_movement_pattern(ctx);

        match movement_pattern {
            MovementPattern::Direct => {
                Some(
                    SteeringBehavior::Arrive {
                        target: target_position,
                        slowing_distance: 10.0,
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity()
                )
            },
            MovementPattern::Curved => {
                // Add curve to movement to lose markers
                let curved_target = self.add_curve_to_path(ctx, target_position);
                Some(
                    SteeringBehavior::Arrive {
                        target: curved_target,
                        slowing_distance: 15.0,
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity()
                )
            },
            MovementPattern::CheckToReceive => {
                // Quick check back toward ball then continue
                let check_position = self.calculate_check_position(ctx);
                Some(
                    SteeringBehavior::Arrive {
                        target: check_position,
                        slowing_distance: 5.0,
                    }
                        .calculate(ctx.player)
                        .velocity
                )
            },
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderCreatingSpaceState {
    /// Calculate intelligent position for creating space
    fn calculate_intelligent_space_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let player_pos = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Identify space type to exploit
        let space_type = self.identify_best_space_type(ctx);

        match space_type {
            SpaceType::HalfSpace => {
                self.move_into_half_space(ctx, field_width, field_height)
            },
            SpaceType::BetweenLines => {
                self.position_between_lines(ctx, field_width, field_height)
            },
            SpaceType::WideOverload => {
                self.create_wide_overload(ctx, field_width, field_height)
            },
            SpaceType::DeepPocket => {
                self.find_deep_pocket(ctx, field_width, field_height)
            },
            SpaceType::ThirdManRun => {
                self.make_third_man_run(ctx, field_width, field_height)
            },
            SpaceType::CentralOverload => {
                self.create_central_overload(ctx, field_width, field_height)
            },
        }
    }

    /// Identify the best type of space to create
    fn identify_best_space_type(&self, ctx: &StateProcessingContext) -> SpaceType {
        let ball_zone = self.get_ball_zone(ctx);
        let team_shape = self.analyze_team_shape(ctx);
        let opponent_shape = self.analyze_opponent_shape(ctx);

        // Decision logic based on game situation
        match ball_zone {
            BallZone::DefensiveThird => {
                if team_shape.is_stretched() {
                    SpaceType::DeepPocket
                } else {
                    SpaceType::BetweenLines
                }
            },
            BallZone::MiddleThird => {
                if opponent_shape.is_compact_central() {
                    SpaceType::WideOverload
                } else if opponent_shape.has_high_line() {
                    SpaceType::ThirdManRun
                } else {
                    SpaceType::HalfSpace
                }
            },
            BallZone::AttackingThird => {
                if opponent_shape.is_narrow() {
                    SpaceType::WideOverload
                } else if self.can_exploit_half_space(ctx) {
                    SpaceType::HalfSpace
                } else {
                    SpaceType::CentralOverload
                }
            },
        }
    }

    /// Move into half-space (between center and wing)
    fn move_into_half_space(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // Half-spaces are roughly at 1/3 and 2/3 of field width
        let left_half_space_y = field_height * 0.33;
        let right_half_space_y = field_height * 0.67;

        // Choose half-space based on ball position and team shape
        let target_y = if ball_pos.y < field_height / 2.0 {
            // Ball on left, potentially move to left half-space or switch
            if self.is_half_space_occupied(ctx, left_half_space_y) {
                right_half_space_y // Switch to opposite half-space
            } else {
                left_half_space_y
            }
        } else {
            if self.is_half_space_occupied(ctx, right_half_space_y) {
                left_half_space_y
            } else {
                right_half_space_y
            }
        };

        // Progressive positioning toward goal
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        let target_x = ball_pos.x + (attacking_direction * 30.0);

        Vector3::new(
            target_x.clamp(10.0, field_width - 10.0),
            target_y,
            0.0
        )
    }

    /// Position between opponent's defensive and midfield lines
    fn position_between_lines(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        let opponent_defenders = ctx.players().opponents().all()
            .filter(|opp| opp.tactical_positions.is_defender())
            .collect::<Vec<_>>();

        let opponent_midfielders = ctx.players().opponents().all()
            .filter(|opp| opp.tactical_positions.is_midfielder())
            .collect::<Vec<_>>();

        if opponent_defenders.is_empty() || opponent_midfielders.is_empty() {
            // Fallback to progressive position
            return self.calculate_progressive_position(ctx, field_width, field_height);
        }

        // Calculate average lines
        let def_line_x = opponent_defenders.iter()
            .map(|d| d.position.x)
            .sum::<f32>() / opponent_defenders.len() as f32;

        let mid_line_x = opponent_midfielders.iter()
            .map(|m| m.position.x)
            .sum::<f32>() / opponent_midfielders.len() as f32;

        // Position between the lines
        let between_x = (def_line_x + mid_line_x) / 2.0;

        // Find lateral space
        let target_y = self.find_lateral_space_between_lines(ctx, &opponent_defenders, &opponent_midfielders);

        Vector3::new(
            between_x.clamp(10.0, field_width - 10.0),
            target_y.clamp(10.0, field_height - 10.0),
            0.0
        )
    }

    /// Create overload on the wing
    fn create_wide_overload(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;

        // Determine which wing needs support
        let left_wing_teammates = ctx.players().teammates().all()
            .filter(|t| t.position.y < field_height * 0.25)
            .count();

        let right_wing_teammates = ctx.players().teammates().all()
            .filter(|t| t.position.y > field_height * 0.75)
            .count();

        // Go to wing with fewer players (to balance) or with more (to overload)
        let overload_strategy = self.determine_overload_strategy(ctx);

        let target_y = match overload_strategy {
            OverloadStrategy::CreateNumericalAdvantage => {
                // Join the stronger wing
                if left_wing_teammates >= right_wing_teammates {
                    field_height * 0.15
                } else {
                    field_height * 0.85
                }
            },
            OverloadStrategy::BalanceWidth => {
                // Go to weaker wing
                if left_wing_teammates <= right_wing_teammates {
                    field_height * 0.15
                } else {
                    field_height * 0.85
                }
            },
        };

        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        Vector3::new(
            ball_pos.x + (attacking_direction * 40.0),
            target_y,
            0.0
        ).clamp_to_field(field_width, field_height)
    }

    /// Find deep pocket of space
    fn find_deep_pocket(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        // Scan for areas with low opponent density
        let grid_size = 30.0;
        let mut best_position = ctx.player.position;
        let mut min_congestion = f32::MAX;

        let scan_range = 100.0;
        let player_pos = ctx.player.position;

        for x in ((player_pos.x - scan_range) as i32..=(player_pos.x + scan_range) as i32).step_by(grid_size as usize) {
            for y in ((player_pos.y - scan_range) as i32..=(player_pos.y + scan_range) as i32).step_by(grid_size as usize) {
                let test_pos = Vector3::new(x as f32, y as f32, 0.0);

                // Skip positions outside field
                if test_pos.x < 10.0 || test_pos.x > field_width - 10.0 ||
                    test_pos.y < 10.0 || test_pos.y > field_height - 10.0 {
                    continue;
                }

                let congestion = self.calculate_position_congestion(ctx, test_pos);

                // Prefer progressive positions
                let progression_bonus = if self.is_progressive_position(ctx, test_pos) {
                    -10.0
                } else {
                    0.0
                };

                let adjusted_congestion = congestion + progression_bonus;

                if adjusted_congestion < min_congestion {
                    min_congestion = adjusted_congestion;
                    best_position = test_pos;
                }
            }
        }

        best_position
    }

    /// Make a third man run (beyond the immediate play)
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
                    0.0
                );

                return beyond_position.clamp_to_field(field_width, field_height);
            }
        }

        // Fallback to progressive run
        self.calculate_progressive_position(ctx, field_width, field_height)
    }

    /// Create central overload
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
            0.0
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

    fn is_half_space_occupied(&self, ctx: &StateProcessingContext, y_position: f32) -> bool {
        ctx.players().teammates().all()
            .any(|t| (t.position.y - y_position).abs() < HALF_SPACE_WIDTH)
    }

    fn can_exploit_half_space(&self, ctx: &StateProcessingContext) -> bool {
        let vision = ctx.player.skills.mental.vision;
        let off_ball = ctx.player.skills.mental.off_the_ball;

        vision > 13.0 && off_ball > 12.0
    }

    fn find_lateral_space_between_lines(
        &self,
        ctx: &StateProcessingContext,
        defenders: &[MatchPlayerLite],
        midfielders: &[MatchPlayerLite]
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
            0.0
        ).clamp_to_field(field_width, field_height)
    }

    fn determine_overload_strategy(&self, ctx: &StateProcessingContext) -> OverloadStrategy {
        // Based on team tactics and game situation
        if ctx.team().is_loosing() {
            OverloadStrategy::CreateNumericalAdvantage
        } else {
            OverloadStrategy::BalanceWidth
        }
    }

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

    fn predict_next_pass_recipient(
        &self,
        ctx: &StateProcessingContext,
        ball_holder: &MatchPlayerLite
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

    fn calculate_run_angle(&self, ctx: &StateProcessingContext, target: &MatchPlayerLite) -> f32 {
        // Angle away from target's current position
        if target.position.y < ctx.context.field_size.height as f32 / 2.0 {
            20.0
        } else {
            -20.0
        }
    }

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

    fn is_opponent_compact_central(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let central_opponents = ctx.players().opponents().all()
            .filter(|o| (o.position.y - field_height / 2.0).abs() < field_height * 0.3)
            .count();

        central_opponents >= 6
    }

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
#[derive(Debug, Clone, Copy)]
enum SpaceType {
    HalfSpace,
    BetweenLines,
    WideOverload,
    DeepPocket,
    ThirdManRun,
    CentralOverload,
}

#[derive(Debug, Clone, Copy)]
enum MovementPattern {
    Direct,
    Curved,
    CheckToReceive,
}

#[derive(Debug, Clone, Copy)]
enum BallZone {
    DefensiveThird,
    MiddleThird,
    AttackingThird,
}

#[derive(Debug, Clone, Copy)]
enum OverloadStrategy {
    CreateNumericalAdvantage,
    BalanceWidth,
}

#[derive(Debug, Default)]
struct TeamShape {
    width: f32,
    depth: f32,
    compactness: f32,
}

impl TeamShape {
    fn is_stretched(&self) -> bool {
        self.depth > 300.0 || self.width > 400.0
    }
}

#[derive(Debug, Default)]
struct OpponentShape {
    high_line: bool,
    compact_central: bool,
    narrow: bool,
}

impl OpponentShape {
    fn is_compact_central(&self) -> bool {
        self.compact_central
    }

    fn has_high_line(&self) -> bool {
        self.high_line
    }

    fn is_narrow(&self) -> bool {
        self.narrow
    }
}

// Extension trait
trait VectorFieldExtensions {
    fn clamp_to_field(self, field_width: f32, field_height: f32) -> Self;
}

impl VectorFieldExtensions for Vector3<f32> {
    fn clamp_to_field(self, field_width: f32, field_height: f32) -> Self {
        Vector3::new(
            self.x.clamp(10.0, field_width - 10.0),
            self.y.clamp(10.0, field_height - 10.0),
            self.z
        )
    }
}