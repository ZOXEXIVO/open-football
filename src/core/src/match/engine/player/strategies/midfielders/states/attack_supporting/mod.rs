use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide,
    StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const TACKLE_RANGE: f32 = 30.0;
const PRESS_RANGE: f32 = 100.0;
const ATTACK_SUPPORT_TIME_LIMIT: u64 = 300;
const CHANNEL_WIDTH: f32 = 15.0; // Width of vertical channels for runs

#[derive(Default)]
pub struct MidfielderAttackSupportingState {}

impl StateProcessingHandler for MidfielderAttackSupportingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If player has the ball, transition to running with ball
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // If team loses possession, switch to defensive duties
        if !ctx.team().is_control_ball() {
            if ctx.ball().distance() < TACKLE_RANGE {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }

            if ctx.ball().distance() < PRESS_RANGE && ctx.ball().is_towards_player_with_angle(0.8) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        // Team has possession - continue supporting
        if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 100.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Check if we should make a late run into the box
        if self.should_make_late_box_run(ctx) {
            // Continue in this state but with more aggressive positioning
            return None;
        }

        // If ball is too far, actively create space
        if ctx.ball().distance() > 300.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::CreatingSpace,
            ));
        }

        // Timeout check
        if ctx.in_state_time > ATTACK_SUPPORT_TIME_LIMIT {
            if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }
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
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance = ctx.ball().distance();

        // Check if we have the ball - if so, drive forward
        if ctx.player.has_ball(ctx) {
            return Some(self.calculate_ball_carrying_velocity(ctx));
        }

        // Key change: Don't run to the ball if a teammate has it
        if let Some(ball_owner_id) = ctx.ball().owner_id() {
            if let Some(ball_owner) = ctx.context.players.by_id(ball_owner_id) {
                if ball_owner.team_id == ctx.player.team_id {
                    // Teammate has ball - make attacking run instead of clustering
                    let target_position = self.calculate_attacking_run_position(ctx);

                    // Vary speed based on situation
                    let urgency_factor = self.calculate_urgency_factor(ctx);
                    let slowing_distance = 20.0 * (1.0 - urgency_factor * 0.3);

                    return Some(
                        SteeringBehavior::Arrive {
                            target: target_position,
                            slowing_distance,
                        }
                            .calculate(ctx.player)
                            .velocity + ctx.player().separation_velocity() * 1.5, // Increase separation
                    );
                }
            }
        }

        // Ball is loose or opponent has it - only pursue if we're closest
        if !ctx.team().is_control_ball() || !ctx.ball().is_owned() {
            if ctx.team().is_best_player_to_chase_ball() && ball_distance < 100.0 {
                // We're best positioned - go get the ball
                return Some(
                    SteeringBehavior::Pursuit {
                        target: ball_position,
                        target_velocity: ctx.tick_context.positions.ball.velocity,
                    }
                        .calculate(ctx.player)
                        .velocity,
                );
            }
        }

        // Default: Make intelligent supporting run
        let target_position = self.calculate_optimal_support_position(ctx);

        // Adjust speed based on urgency
        let urgency_factor = self.calculate_urgency_factor(ctx);
        let slowing_distance = 30.0 * (1.0 - urgency_factor * 0.5);

        Some(
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Attack supporting is high intensity - sustained running to support attacks
        MidfielderCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl MidfielderAttackSupportingState {
    // Add new helper method for attacking runs when teammate has ball
    fn calculate_attacking_run_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let goal_position = ctx.player().opponent_goal_position();
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine attacking direction
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        let distance_to_goal = (ball_position - goal_position).magnitude();

        // Different run types based on position and situation
        let run_type = self.determine_run_type(ctx, distance_to_goal);

        match run_type {
            AttackingRunType::ThroughBall => {
                // Run beyond the defensive line toward goal
                let advanced_position = Vector3::new(
                    goal_position.x - (attacking_direction * 120.0),
                    player_position.y + self.calculate_lateral_run_adjustment(ctx),
                    0.0,
                );

                // Check offside risk and adjust
                if self.is_offside_risk(ctx, advanced_position) {
                    Vector3::new(
                        advanced_position.x - (attacking_direction * 20.0),
                        advanced_position.y,
                        0.0,
                    ).clamp_to_field(field_width, field_height)
                } else {
                    advanced_position.clamp_to_field(field_width, field_height)
                }
            }
            AttackingRunType::OverlapRun => {
                // Wide overlapping run
                let side_adjustment = if player_position.y < field_height / 2.0 {
                    -field_height * 0.35  // Go to left flank
                } else {
                    field_height * 0.35   // Go to right flank
                };

                Vector3::new(
                    ball_position.x + (attacking_direction * 60.0),
                    field_height / 2.0 + side_adjustment,
                    0.0,
                ).clamp_to_field(field_width, field_height)
            }
            AttackingRunType::LateBoxRun => {
                // Late run into the box
                let box_entry_point = self.find_box_entry_point(ctx, goal_position);
                box_entry_point.clamp_to_field(field_width, field_height)
            }
            AttackingRunType::SupportRun => {
                // Supporting run to create passing option
                let support_angle = if player_position.y < ball_position.y {
                    -30.0_f32.to_radians()
                } else {
                    30.0_f32.to_radians()
                };

                let support_distance = 40.0;
                let support_offset = Vector3::new(
                    support_distance * support_angle.cos() * attacking_direction,
                    support_distance * support_angle.sin(),
                    0.0,
                );

                (ball_position + support_offset).clamp_to_field(field_width, field_height)
            }
            AttackingRunType::DiagonalRun => {
                // Diagonal run to exploit space between defenders
                let diagonal_target = Vector3::new(
                    ball_position.x + (attacking_direction * 70.0),
                    player_position.y + if player_position.y < field_height / 2.0 { 40.0 } else { -40.0 },
                    0.0,
                );

                diagonal_target.clamp_to_field(field_width, field_height)
            }
        }
    }

    // Add new helper to determine run type
    fn determine_run_type(&self, ctx: &StateProcessingContext, distance_to_goal: f32) -> AttackingRunType {
        let field_width = ctx.context.field_size.width as f32;
        let player_skills = &ctx.player.skills;

        // Player attributes affect run selection
        let pace = player_skills.physical.pace;
        let off_the_ball = player_skills.mental.off_the_ball;
        let anticipation = player_skills.mental.anticipation;

        // Close to goal - make decisive runs
        if distance_to_goal < field_width * 0.25 {
            if off_the_ball > 14.0 && pace > 14.0 {
                AttackingRunType::ThroughBall
            } else if anticipation > 13.0 {
                AttackingRunType::LateBoxRun
            } else {
                AttackingRunType::SupportRun
            }
        }
        // Middle third - varied runs
        else if distance_to_goal < field_width * 0.5 {
            let has_space_wide = self.check_wide_space(ctx);

            if has_space_wide && pace > 13.0 {
                AttackingRunType::OverlapRun
            } else if off_the_ball > 12.0 {
                AttackingRunType::DiagonalRun
            } else {
                AttackingRunType::SupportRun
            }
        }
        // Build-up phase - support play
        else {
            AttackingRunType::SupportRun
        }
    }

    // Add helper to calculate lateral adjustment for runs
    fn calculate_lateral_run_adjustment(&self, ctx: &StateProcessingContext) -> f32 {
        let field_height = ctx.context.field_size.height as f32;
        let player_y = ctx.player.position.y;

        // Check defender positioning
        let defenders_central = ctx.players().opponents().all()
            .filter(|opp| {
                opp.tactical_positions.is_defender() &&
                    (opp.position.y - field_height / 2.0).abs() < field_height * 0.2
            })
            .count();

        // If defenders are concentrated centrally, make wider runs
        if defenders_central >= 2 {
            if player_y < field_height / 2.0 {
                -30.0  // Go wider left
            } else {
                30.0   // Go wider right
            }
        } else {
            // Make central runs if space exists
            if (player_y - field_height / 2.0).abs() > field_height * 0.25 {
                if player_y < field_height / 2.0 {
                    20.0   // Come inside from left
                } else {
                    -20.0  // Come inside from right
                }
            } else {
                0.0
            }
        }
    }

    // Add helper to find best box entry point
    fn find_box_entry_point(&self, ctx: &StateProcessingContext, goal_position: Vector3<f32>) -> Vector3<f32> {
        let field_height = ctx.context.field_size.height as f32;

        // Identify gaps in the box
        let box_defenders = ctx.players().opponents().all()
            .filter(|opp| {
                let dist_to_goal = (opp.position - goal_position).magnitude();
                dist_to_goal < 200.0 && opp.tactical_positions.is_defender()
            })
            .collect::<Vec<_>>();

        // Find best entry point based on defender positions
        if box_defenders.is_empty() {
            // No defenders - go straight to goal
            Vector3::new(
                goal_position.x - 100.0,
                goal_position.y,
                0.0,
            )
        } else {
            // Find gap between defenders
            let mut best_gap_y = goal_position.y;
            let mut max_gap_size = 0.0;

            for window in box_defenders.windows(2) {
                let gap_y = (window[0].position.y + window[1].position.y) / 2.0;
                let gap_size = (window[1].position.y - window[0].position.y).abs();

                if gap_size > max_gap_size {
                    max_gap_size = gap_size;
                    best_gap_y = gap_y;
                }
            }

            // Also check edges
            let edge_gap_top = field_height * 0.35 - box_defenders.first().map(|d| d.position.y).unwrap_or(0.0);
            let edge_gap_bottom = field_height * 0.65 - box_defenders.last().map(|d| d.position.y).unwrap_or(field_height);

            if edge_gap_top > max_gap_size {
                best_gap_y = goal_position.y - 80.0;
            } else if edge_gap_bottom > max_gap_size {
                best_gap_y = goal_position.y + 80.0;
            }

            Vector3::new(
                goal_position.x - 150.0,
                best_gap_y,
                0.0,
            )
        }
    }

    // Add helper to check wide space availability
    fn check_wide_space(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let player_y = ctx.player.position.y;

        // Determine which flank to check
        let flank_y = if player_y < field_height / 2.0 {
            field_height * 0.15  // Left flank
        } else {
            field_height * 0.85  // Right flank
        };

        // Count opponents in wide area
        let opponents_wide = ctx.players().opponents().all()
            .filter(|opp| (opp.position.y - flank_y).abs() < 30.0)
            .count();

        opponents_wide < 2
    }

    // Add method for ball carrying when midfielder has possession
    fn calculate_ball_carrying_velocity(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_position = ctx.player().opponent_goal_position();
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Check pressure
        let under_pressure = ctx.players().opponents().exists(15.0);

        if under_pressure {
            // Under pressure - make quick decision
            if ctx.player().has_clear_shot() && ctx.ball().distance_to_opponent_goal() < 250.0 {
                // Face goal for shot
                let to_goal = (goal_position - player_position).normalize();
                return to_goal * 2.0;
            }

            // Look for outlet pass by turning away from pressure
            let nearest_opponent = ctx.players().opponents().nearby(15.0).next();
            if let Some(opponent) = nearest_opponent {
                let away_from_pressure = (player_position - opponent.position).normalize();
                return away_from_pressure * 3.0;
            }
        }

        // Not under immediate pressure - drive forward intelligently
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        // Find space to drive into
        let forward_space = Vector3::new(
            player_position.x + (attacking_direction * 40.0),
            player_position.y,
            0.0,
        );

        // Check if forward space is clear
        let forward_clear = !ctx.players().opponents().all()
            .any(|opp| (opp.position - forward_space).magnitude() < 20.0);

        if forward_clear {
            // Drive forward with pace
            let drive_speed = ctx.player.skills.physical.pace * 0.35;
            SteeringBehavior::Seek {
                target: goal_position,
            }
                .calculate(ctx.player)
                .velocity * (drive_speed / ctx.player.skills.max_speed_with_condition(
                    ctx.player.player_attributes.condition,
                ))
        } else {
            // Space blocked - move laterally to find space
            let lateral_target = Vector3::new(
                player_position.x + (attacking_direction * 20.0),
                if player_position.y < field_height / 2.0 {
                    player_position.y + 30.0
                } else {
                    player_position.y - 30.0
                },
                0.0,
            ).clamp_to_field(field_width, field_height);

            SteeringBehavior::Arrive {
                target: lateral_target,
                slowing_distance: 10.0,
            }
                .calculate(ctx.player)
                .velocity
        }
    }

    /// Calculate the optimal position to support the attack
    fn calculate_optimal_support_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let _player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine attacking direction
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => 1.0,
            Some(PlayerSide::Right) => -1.0,
            None => 0.0,
        };

        let goal_position = ctx.player().opponent_goal_position();
        let distance_to_goal = (ball_position - goal_position).magnitude();

        // Different support strategies based on attacking phase
        if distance_to_goal < field_width * 0.25 {
            // Final third - make late runs into the box
            self.calculate_late_box_run_position(ctx, attacking_direction, field_width, field_height)
        } else if distance_to_goal < field_width * 0.5 {
            // Middle attacking third - create passing triangles and support wide
            self.calculate_middle_third_support(ctx, attacking_direction, field_width, field_height)
        } else {
            // Build-up phase - provide passing options
            self.calculate_buildup_support_position(ctx, attacking_direction, field_width, field_height)
        }
    }

    /// Calculate position for late runs into the box
    fn calculate_late_box_run_position(
        &self,
        ctx: &StateProcessingContext,
        attacking_direction: f32,
        field_width: f32,
        field_height: f32,
    ) -> Vector3<f32> {
        let _ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let goal_position = ctx.player().opponent_goal_position();

        // Identify free channels between defenders
        let channels = self.identify_free_channels(ctx, goal_position);

        if let Some(best_channel) = channels.first() {
            // Run into the free channel
            let target_x = goal_position.x - (attacking_direction * 150.0);
            let target_y = best_channel.center_y;

            // Add slight curve to the run to stay onside
            let curve_factor = if self.is_offside_risk(ctx, Vector3::new(target_x, target_y, 0.0)) {
                -20.0 * attacking_direction
            } else {
                0.0
            };

            return Vector3::new(
                target_x + curve_factor,
                target_y,
                0.0,
            ).clamp_to_field(field_width, field_height);
        }

        // Default: Edge of the box for cutback opportunities
        let box_edge_x = goal_position.x - (attacking_direction * 180.0);
        let box_edge_y = if player_position.y < field_height / 2.0 {
            goal_position.y - 100.0
        } else {
            goal_position.y + 100.0
        };

        Vector3::new(box_edge_x, box_edge_y, 0.0).clamp_to_field(field_width, field_height)
    }

    /// Calculate support position in middle third
    fn calculate_middle_third_support(
        &self,
        ctx: &StateProcessingContext,
        attacking_direction: f32,
        field_width: f32,
        field_height: f32,
    ) -> Vector3<f32> {
        // Check where attacking teammates are
        let attacking_players = self.get_attacking_teammates(ctx);

        // Create triangles with ball carrier and forwards
        if let Some(ball_holder) = self.find_ball_holder(ctx) {
            // Position to create a passing triangle
            let triangle_position = self.create_passing_triangle(
                ctx,
                &ball_holder,
                &attacking_players,
                attacking_direction,
            );

            if self.is_position_valuable(ctx, triangle_position) {
                return triangle_position.clamp_to_field(field_width, field_height);
            }
        }

        // Support wide if center is congested
        if self.is_center_congested(ctx) {
            let wide_position = self.calculate_wide_support(ctx, attacking_direction);
            return wide_position.clamp_to_field(field_width, field_height);
        }

        // Default: Position between lines
        self.position_between_lines(ctx, attacking_direction)
            .clamp_to_field(field_width, field_height)
    }

    /// Calculate support position during build-up
    fn calculate_buildup_support_position(
        &self,
        ctx: &StateProcessingContext,
        attacking_direction: f32,
        field_width: f32,
        field_height: f32,
    ) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;

        // Provide a progressive passing option
        let progressive_position = Vector3::new(
            ball_position.x + (attacking_direction * 80.0),
            ball_position.y + self.calculate_lateral_movement(ctx),
            0.0,
        );

        // Ensure we're not too close to other midfielders
        let adjusted_position = self.avoid_midfielder_clustering(ctx, progressive_position);

        adjusted_position.clamp_to_field(field_width, field_height)
    }

    /// Identify free channels between defenders
    fn identify_free_channels(&self, ctx: &StateProcessingContext, goal_position: Vector3<f32>) -> Vec<Channel> {
        let mut channels = Vec::new();
        let defenders = ctx.players().opponents().all()
            .filter(|opp| opp.tactical_positions.is_defender())
            .collect::<Vec<_>>();

        if defenders.len() < 2 {
            // If few defenders, the whole width is available
            channels.push(Channel {
                center_y: goal_position.y,
                width: 30.0,
                congestion: 0.0,
            });
            return channels;
        }

        // Sort defenders by Y position
        let mut sorted_defenders = defenders.clone();
        sorted_defenders.sort_by(|a, b|
            a.position.y.partial_cmp(&b.position.y).unwrap_or(std::cmp::Ordering::Equal)
        );

        // Find gaps between defenders
        for window in sorted_defenders.windows(2) {
            let gap = (window[1].position.y - window[0].position.y).abs();
            if gap > CHANNEL_WIDTH {
                channels.push(Channel {
                    center_y: (window[0].position.y + window[1].position.y) / 2.0,
                    width: gap,
                    congestion: self.calculate_channel_congestion(ctx, window[0].position, window[1].position),
                });
            }
        }

        // Sort by least congested
        channels.sort_by(|a, b|
            a.congestion.partial_cmp(&b.congestion).unwrap_or(std::cmp::Ordering::Equal)
        );

        channels
    }

    /// Check if position risks being offside
    fn is_offside_risk(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        let last_defender = ctx.players().opponents().all()
            .filter(|opp| !opp.tactical_positions.is_goalkeeper())
            .min_by(|a, b| {
                let a_x = match ctx.player.side {
                    Some(PlayerSide::Left) => a.position.x,
                    Some(PlayerSide::Right) => -a.position.x,
                    None => 0.0,
                };
                let b_x = match ctx.player.side {
                    Some(PlayerSide::Left) => b.position.x,
                    Some(PlayerSide::Right) => -b.position.x,
                    None => 0.0,
                };
                b_x.partial_cmp(&a_x).unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some(defender) = last_defender {
            match ctx.player.side {
                Some(PlayerSide::Left) => position.x > defender.position.x + 5.0,
                Some(PlayerSide::Right) => position.x < defender.position.x - 5.0,
                None => false,
            }
        } else {
            false
        }
    }

    /// Check if should make a late run into the box
    fn should_make_late_box_run(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let field_width = ctx.context.field_size.width as f32;

        // Check conditions for late run
        distance_to_goal < field_width * 0.3 &&
            ctx.team().is_control_ball() &&
            !self.is_offside_risk(ctx, ctx.player.position) &&
            ctx.player.skills.mental.off_the_ball > 12.0
    }

    /// Create a passing triangle position
    fn create_passing_triangle(
        &self,
        ctx: &StateProcessingContext,
        ball_holder: &MatchPlayerLite,
        attacking_players: &[MatchPlayerLite],
        attacking_direction: f32,
    ) -> Vector3<f32> {
        let ball_holder_pos = ball_holder.position;

        // Find the most advanced attacker
        let forward = attacking_players.iter()
            .max_by(|a, b| {
                let a_advance = a.position.x * attacking_direction;
                let b_advance = b.position.x * attacking_direction;
                a_advance.partial_cmp(&b_advance).unwrap_or(std::cmp::Ordering::Equal)
            });

        if let Some(forward) = forward {
            // Position to create triangle
            let midpoint = (ball_holder_pos + forward.position) * 0.5;
            let perpendicular = Vector3::new(
                0.0,
                if midpoint.y < ctx.context.field_size.height as f32 / 2.0 { 30.0 } else { -30.0 },
                0.0,
            );

            return midpoint + perpendicular;
        }

        // Default progressive position
        ball_holder_pos + Vector3::new(attacking_direction * 40.0, 20.0, 0.0)
    }

    /// Get attacking teammates
    fn get_attacking_teammates(&self, ctx: &StateProcessingContext) -> Vec<MatchPlayerLite> {
        ctx.players().teammates().all()
            .filter(|t| t.tactical_positions.is_forward() ||
                (t.tactical_positions.is_midfielder() &&
                    self.is_in_attacking_position(ctx, t)))
            .collect()
    }

    /// Check if a position is valuable for attack
    fn is_position_valuable(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        // Not too crowded
        let opponents_nearby = ctx.players().opponents().all()
            .filter(|opp| (opp.position - position).magnitude() < 15.0)
            .count();

        // Has passing options
        let teammates_in_range = ctx.players().teammates().all()
            .filter(|t| {
                let dist = (t.position - position).magnitude();
                dist > 20.0 && dist < 60.0
            })
            .count();

        opponents_nearby < 2 && teammates_in_range >= 2
    }

    /// Check if center is congested
    fn is_center_congested(&self, ctx: &StateProcessingContext) -> bool {
        let field_height = ctx.context.field_size.height as f32;
        let center_y = field_height / 2.0;
        let ball_position = ctx.tick_context.positions.ball.position;

        let players_in_center = ctx.players().opponents().all()
            .filter(|opp| {
                (opp.position.y - center_y).abs() < field_height * 0.2 &&
                    (opp.position.x - ball_position.x).abs() < 50.0
            })
            .count();

        players_in_center >= 3
    }

    /// Calculate wide support position
    fn calculate_wide_support(&self, ctx: &StateProcessingContext, attacking_direction: f32) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let field_height = ctx.context.field_size.height as f32;

        // Determine which flank is less occupied
        let left_flank_players = ctx.players().teammates().all()
            .filter(|t| t.position.y < field_height * 0.3)
            .count();

        let right_flank_players = ctx.players().teammates().all()
            .filter(|t| t.position.y > field_height * 0.7)
            .count();

        let target_y = if left_flank_players <= right_flank_players {
            field_height * 0.15
        } else {
            field_height * 0.85
        };

        Vector3::new(
            ball_position.x + (attacking_direction * 50.0),
            target_y,
            0.0,
        )
    }

    /// Position between defensive lines
    fn position_between_lines(&self, ctx: &StateProcessingContext, attacking_direction: f32) -> Vector3<f32> {
        let defenders = ctx.players().opponents().all()
            .filter(|opp| opp.tactical_positions.is_defender())
            .collect::<Vec<_>>();

        let midfielders = ctx.players().opponents().all()
            .filter(|opp| opp.tactical_positions.is_midfielder())
            .collect::<Vec<_>>();

        if !defenders.is_empty() && !midfielders.is_empty() {
            let avg_def_x = defenders.iter().map(|d| d.position.x).sum::<f32>() / defenders.len() as f32;
            let avg_mid_x = midfielders.iter().map(|m| m.position.x).sum::<f32>() / midfielders.len() as f32;

            let between_x = (avg_def_x + avg_mid_x) / 2.0;
            let player_y = ctx.player.position.y;

            return Vector3::new(between_x, player_y, 0.0);
        }

        // Default progressive position
        ctx.player.position + Vector3::new(attacking_direction * 40.0, 0.0, 0.0)
    }

    /// Calculate lateral movement to create space
    fn calculate_lateral_movement(&self, ctx: &StateProcessingContext) -> f32 {
        let field_height = ctx.context.field_size.height as f32;
        let player_y = ctx.player.position.y;
        let center_y = field_height / 2.0;

        // Move away from crowded areas
        let crowd_factor = self.calculate_crowd_factor(ctx, ctx.player.position);

        if crowd_factor > 0.5 {
            // Move toward less crowded flank
            if player_y < center_y {
                -30.0
            } else {
                30.0
            }
        } else {
            // Maintain width
            if (player_y - center_y).abs() < field_height * 0.2 {
                if player_y < center_y { -20.0 } else { 20.0 }
            } else {
                0.0
            }
        }
    }

    /// Avoid clustering with other midfielders
    fn avoid_midfielder_clustering(&self, ctx: &StateProcessingContext, target: Vector3<f32>) -> Vector3<f32> {
        let other_midfielders = ctx.players().teammates().all()
            .filter(|t| t.tactical_positions.is_midfielder() && t.id != ctx.player.id)
            .collect::<Vec<_>>();

        let mut adjusted = target;

        for midfielder in other_midfielders {
            let distance = (midfielder.position - adjusted).magnitude();
            if distance < 25.0 {
                // Move away from clustered midfielder
                let away = (adjusted - midfielder.position).normalize();
                adjusted = adjusted + away * (25.0 - distance);
            }
        }

        adjusted
    }

    /// Calculate urgency factor for movement
    fn calculate_urgency_factor(&self, ctx: &StateProcessingContext) -> f32 {
        let mut urgency: f32 = 0.5;

        // Increase urgency if team is losing
        if ctx.team().is_loosing() {
            urgency += 0.2;
        }

        // Increase urgency late in game
        if ctx.context.time.is_running_out() {
            urgency += 0.2;
        }

        // Increase urgency if good attacking opportunity
        if ctx.ball().distance_to_opponent_goal() < 200.0 {
            urgency += 0.1;
        }

        urgency.min(1.0)
    }

    /// Calculate crowd factor around a position
    fn calculate_crowd_factor(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> f32 {
        let players_nearby = ctx.players().opponents().all()
            .chain(ctx.players().teammates().all())
            .filter(|p| (p.position - position).magnitude() < 30.0)
            .count();

        (players_nearby as f32 / 8.0).min(1.0)
    }

    /// Calculate channel congestion
    fn calculate_channel_congestion(
        &self,
        ctx: &StateProcessingContext,
        pos1: Vector3<f32>,
        pos2: Vector3<f32>,
    ) -> f32 {
        let center = (pos1 + pos2) * 0.5;
        let players_in_channel = ctx.players().opponents().all()
            .filter(|opp| {
                let dist_to_center = (opp.position - center).magnitude();
                dist_to_center < 20.0
            })
            .count();

        players_in_channel as f32 / 3.0
    }

    /// Check if player is in attacking position
    fn is_in_attacking_position(&self, ctx: &StateProcessingContext, player: &MatchPlayerLite) -> bool {
        let field_width = ctx.context.field_size.width as f32;
        match ctx.player.side {
            Some(PlayerSide::Left) => player.position.x > field_width * 0.6,
            Some(PlayerSide::Right) => player.position.x < field_width * 0.4,
            None => false,
        }
    }

    /// Find teammate who currently has the ball
    fn find_ball_holder(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        if let Some(owner_id) = ctx.ball().owner_id() {
            if let Some(owner) = ctx.context.players.by_id(owner_id) {
                if owner.team_id == ctx.player.team_id {
                    return Some(MatchPlayerLite {
                        id: owner_id,
                        position: ctx.tick_context.positions.players.position(owner_id),
                        tactical_positions: owner.tactical_position.current_position,
                    });
                }
            }
        }
        None
    }
}

#[derive(Debug, Clone, Copy)]
enum AttackingRunType {
    ThroughBall,   // Run behind defensive line
    OverlapRun,    // Wide overlapping run
    LateBoxRun,    // Late run into penalty area
    SupportRun,    // Supporting run for passing option
    DiagonalRun,   // Diagonal run to exploit space
}

/// Channel between defenders
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Channel {
    center_y: f32,
    width: f32,
    congestion: f32,
}

/// Extension trait for Vector3 to clamp to field
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