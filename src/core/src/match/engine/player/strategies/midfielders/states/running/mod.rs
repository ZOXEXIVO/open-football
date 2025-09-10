use crate::IntegerUtils;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 300.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 10.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

#[derive(Default)]
pub struct MidfielderRunningState {}

impl StateProcessingHandler for MidfielderRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            if self.has_clear_shot(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            if self.in_long_distance_shooting_range(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::DistanceShooting,
                ));
            }

            if self.in_shooting_range(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Shooting,
                ));
            }

            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }

            if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Dribbling,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        } else {
            if self.should_intercept(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Intercepting,
                ));
            }

            if self.should_press(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            if self.should_support_attack(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::AttackSupporting,
                ));
            }

            if self.should_return_to_position(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Passing,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Priority 1: Follow waypoints when appropriate
        if ctx.player.should_follow_waypoints(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: IntegerUtils::random(1, 10) as f32,
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity(),
                );
            }
        }

        // Priority 2: If player has the ball, use intelligent movement
        if ctx.player.has_ball(ctx) {
            return Some(self.calculate_ball_carrying_velocity(ctx));
        }

        // Priority 3: Without ball, move to support play or find space
        if ctx.team().is_control_ball() {
            return Some(self.calculate_support_velocity(ctx));
        }

        // Priority 4: Defensive positioning
        Some(self.calculate_defensive_velocity(ctx))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderRunningState {
    /// Calculate velocity when carrying the ball - more intelligent movement
    fn calculate_ball_carrying_velocity(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let goal_position = ctx.player().opponent_goal_position();
        let player_position = ctx.player.position;

        // Get player's mental attributes for decision making
        let vision = ctx.player.skills.mental.vision / 20.0;
        let creativity = ctx.player.skills.mental.flair / 20.0;
        let decisions = ctx.player.skills.mental.decisions / 20.0;

        // Calculate different movement options
        let mut movement_options: Vec<(Vector3<f32>, f32)> = Vec::new();

        // Option 1: Direct to goal (but not straight line)
        let direct_angle_variation = (rand::random::<f32>() - 0.5) * 30.0_f32.to_radians();
        let to_goal = (goal_position - player_position).normalize();
        let rotated_to_goal = self.rotate_vector_2d(to_goal, direct_angle_variation);
        let direct_score = self.evaluate_direction(ctx, player_position + rotated_to_goal * 50.0);
        movement_options.push((rotated_to_goal, direct_score * (1.0 - creativity * 0.3)));

        // Option 2: Wide movement (use flanks)
        let field_center_y = field_height / 2.0;
        let distance_from_center = (player_position.y - field_center_y).abs();

        if distance_from_center < field_height * 0.3 {
            // Player is central, consider moving wide
            let wide_direction = if player_position.y < field_center_y {
                Vector3::new(0.5, -0.8, 0.0).normalize() // Move to left flank
            } else {
                Vector3::new(0.5, 0.8, 0.0).normalize() // Move to right flank
            };

            let wide_target = player_position + wide_direction * 40.0;
            if self.is_position_valid(wide_target, field_width, field_height) {
                let wide_score = self.evaluate_direction(ctx, wide_target);
                movement_options.push((wide_direction, wide_score * (1.0 + creativity * 0.2)));
            }
        }

        // Option 3: Cut inside (if on flanks)
        if distance_from_center > field_height * 0.25 {
            let cut_inside_direction = Vector3::new(
                0.7,
                if player_position.y < field_center_y { 0.3 } else { -0.3 },
                0.0
            ).normalize();

            let cut_inside_target = player_position + cut_inside_direction * 40.0;
            if self.is_position_valid(cut_inside_target, field_width, field_height) {
                let cut_score = self.evaluate_direction(ctx, cut_inside_target);
                movement_options.push((cut_inside_direction, cut_score * (1.0 + vision * 0.2)));
            }
        }

        // Option 4: Find space between opponents
        if let Some(space_target) = self.find_space_between_opponents(ctx) {
            let to_space = (space_target - player_position).normalize();
            let space_score = self.evaluate_direction(ctx, space_target);
            movement_options.push((to_space, space_score * (1.0 + decisions * 0.3)));
        }

        // Option 5: Diagonal runs
        let diagonal_options = vec![
            Vector3::new(0.7, 0.3, 0.0),
            Vector3::new(0.7, -0.3, 0.0),
            Vector3::new(0.5, 0.5, 0.0),
            Vector3::new(0.5, -0.5, 0.0),
        ];

        for diagonal in diagonal_options {
            let diagonal_dir = diagonal.normalize();
            let diagonal_target = player_position + diagonal_dir * 35.0;

            if self.is_position_valid(diagonal_target, field_width, field_height) {
                let diagonal_score = self.evaluate_direction(ctx, diagonal_target);
                movement_options.push((diagonal_dir, diagonal_score));
            }
        }

        // Select best option based on scores
        let best_option = movement_options
            .into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|opt| opt.0)
            .unwrap_or(to_goal);

        // Calculate dynamic speed based on situation
        let base_speed = ctx.player.skills.physical.pace * 0.3;
        let dribbling_skill = ctx.player.skills.technical.dribbling / 20.0;

        // Adjust speed based on pressure
        let pressure_factor = if ctx.players().opponents().exists(15.0) {
            0.7 // Slow down when under pressure
        } else if ctx.players().opponents().exists(25.0) {
            0.85
        } else {
            1.0 // Full speed in open space
        };

        let final_speed = base_speed * (0.6 + dribbling_skill * 0.4) * pressure_factor;

        // Add some natural movement variation
        let movement_noise = Vector3::new(
            (rand::random::<f32>() - 0.5) * 0.1,
            (rand::random::<f32>() - 0.5) * 0.1,
            0.0
        );

        let final_direction = (best_option + movement_noise).normalize();

        // Return velocity with separation
        SteeringBehavior::Arrive {
            target: player_position + final_direction * 40.0,
            slowing_distance: 20.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    /// Calculate velocity when supporting team in possession
    fn calculate_support_velocity(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Find open spaces to support
        if let Some(support_position) = self.find_support_position(ctx) {
            return SteeringBehavior::Arrive {
                target: support_position,
                slowing_distance: 15.0,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity();
        }

        // Default: Move into space ahead
        let forward_space = self.find_forward_space(ctx, field_width, field_height);

        SteeringBehavior::Arrive {
            target: forward_space,
            slowing_distance: 20.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    /// Calculate defensive positioning velocity
    fn calculate_defensive_velocity(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Balance between returning to position and tracking threats
        let to_start = ctx.player.start_position - ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;
        let to_ball = ball_position - ctx.player.position;

        // Weight based on ball distance
        let ball_distance = to_ball.magnitude();
        let position_weight = if ball_distance > 200.0 {
            0.7 // Prioritize position when ball is far
        } else if ball_distance > 100.0 {
            0.5
        } else {
            0.3 // Prioritize ball when it's close
        };

        let combined_target = ctx.player.position +
            (to_start.normalize() * position_weight +
                to_ball.normalize() * (1.0 - position_weight)) * 30.0;

        SteeringBehavior::Arrive {
            target: combined_target,
            slowing_distance: 15.0,
        }
            .calculate(ctx.player)
            .velocity + ctx.player().separation_velocity()
    }

    /// Evaluate a potential movement direction
    fn evaluate_direction(&self, ctx: &StateProcessingContext, target: Vector3<f32>) -> f32 {
        let mut score = 10.0;

        // Check opponent density at target
        let opponents_near_target = ctx.players().opponents().all()
            .filter(|opp| (opp.position - target).magnitude() < 20.0)
            .count();
        score -= opponents_near_target as f32 * 3.0;

        // Bonus for moving toward goal
        let current_goal_dist = ctx.ball().distance_to_opponent_goal();
        let target_goal_dist = (target - ctx.player().opponent_goal_position()).magnitude();
        if target_goal_dist < current_goal_dist {
            score += 2.0;
        }

        // Bonus for open space
        if !ctx.players().opponents().all()
            .any(|opp| (opp.position - target).magnitude() < 15.0) {
            score += 3.0;
        }

        score.max(0.0)
    }

    /// Find a good support position when team has possession
    fn find_support_position(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Create triangle with ball carrier
        let angle_options: Vec<f32> = vec![45.0, -45.0, 60.0, -60.0, 30.0, -30.0];
        let support_distance = 30.0;

        for angle_deg in angle_options {
            let angle_rad = angle_deg.to_radians();
            let support_offset = Vector3::new(
                angle_rad.cos() * support_distance,
                angle_rad.sin() * support_distance,
                0.0
            );

            let potential_position = ball_position + support_offset;

            if self.is_position_valid(potential_position, field_width, field_height) &&
                !self.is_position_occupied(ctx, potential_position) {
                return Some(potential_position);
            }
        }

        None
    }

    /// Find forward space to move into
    fn find_forward_space(&self, ctx: &StateProcessingContext, field_width: f32, field_height: f32) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let attacking_direction = match ctx.player.side {
            Some(PlayerSide::Left) => Vector3::new(1.0, 0.0, 0.0),
            Some(PlayerSide::Right) => Vector3::new(-1.0, 0.0, 0.0),
            None => Vector3::new(0.0, 0.0, 0.0),
        };

        // Look for space diagonally forward
        let lateral_offset = if player_position.y < field_height / 2.0 {
            Vector3::new(0.0, 20.0, 0.0)
        } else {
            Vector3::new(0.0, -20.0, 0.0)
        };

        let target = player_position + attacking_direction * 40.0 + lateral_offset;

        // Constrain to field
        Vector3::new(
            target.x.clamp(10.0, field_width - 10.0),
            target.y.clamp(10.0, field_height - 10.0),
            0.0
        )
    }

    /// Check if a position is occupied by teammates
    fn is_position_occupied(&self, ctx: &StateProcessingContext, position: Vector3<f32>) -> bool {
        ctx.players().teammates().all()
            .any(|teammate| (teammate.position - position).magnitude() < 10.0)
    }

    /// Validate position is within field bounds
    fn is_position_valid(&self, position: Vector3<f32>, field_width: f32, field_height: f32) -> bool {
        position.x >= 5.0 && position.x <= field_width - 5.0 &&
            position.y >= 5.0 && position.y <= field_height - 5.0
    }

    /// Rotate a 2D vector by an angle
    fn rotate_vector_2d(&self, vec: Vector3<f32>, angle_rad: f32) -> Vector3<f32> {
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();

        Vector3::new(
            vec.x * cos_a - vec.y * sin_a,
            vec.x * sin_a + vec.y * cos_a,
            0.0
        )
    }

    // Keep existing helper methods unchanged
    fn in_long_distance_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        (MIN_SHOOTING_DISTANCE..=MAX_SHOOTING_DISTANCE)
            .contains(&ctx.ball().distance_to_opponent_goal())
    }

    fn in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        (MIN_SHOOTING_DISTANCE..=MAX_SHOOTING_DISTANCE)
            .contains(&ctx.ball().distance_to_opponent_goal())
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().distance_to_opponent_goal() < MAX_SHOOTING_DISTANCE {
            return ctx.player().has_clear_shot();
        }

        false
    }

    fn should_intercept(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().is_owned() {
            return false;
        }

        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return true;
        }

        if ctx.ball().distance() < 100.0 {
            return true;
        }

        false
    }

    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        let pressing_distance = 100.0;

        !ctx.team().is_control_ball()
            && ctx.ball().distance() < pressing_distance
            && ctx.ball().is_towards_player_with_angle(0.8)
    }

    pub fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.players().opponents().exists(15.0) {
            return true;
        }

        let game_vision_threshold = 14.0;

        if ctx.player.skills.mental.vision >= game_vision_threshold {
            return self.find_open_teammate_on_opposite_side(ctx).is_some();
        }

        false
    }

    fn find_open_teammate_on_opposite_side(
        &self,
        ctx: &StateProcessingContext,
    ) -> Option<MatchPlayerLite> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let opposite_side_x = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.75,
            Some(PlayerSide::Right) => field_width * 0.25,
            None => return None,
        };

        let mut open_teammates: Vec<MatchPlayerLite> = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .filter(|teammate| {
                let is_on_opposite_side = match ctx.player.side {
                    Some(PlayerSide::Left) => teammate.position.x > opposite_side_x,
                    Some(PlayerSide::Right) => teammate.position.x < opposite_side_x,
                    None => false,
                };
                let is_open = !ctx
                    .players()
                    .opponents()
                    .nearby(20.0)
                    .any(|opponent| opponent.id == teammate.id);

                is_on_opposite_side && is_open
            })
            .collect();

        if open_teammates.is_empty() {
            None
        } else {
            open_teammates.sort_by(|a, b| {
                let dist_a = (a.position - player_position).magnitude();
                let dist_b = (b.position - player_position).magnitude();
                dist_a.partial_cmp(&dist_b).unwrap()
            });
            Some(open_teammates[0])
        }
    }

    fn find_space_between_opponents(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let players = ctx.players();
        let opponents = players.opponents();

        let mut nearest_opponents = opponents.nearby_raw(200.0);

        if let Some((first_id, _)) = nearest_opponents.next() {
            while let Some((second_id, _)) = nearest_opponents.next() {
                if first_id == second_id {
                    continue;
                }
                let distance_between_opponents =
                    ctx.tick_context.distances.get(first_id, second_id);
                if distance_between_opponents > 10.0 {
                    let first_position = ctx.tick_context.positions.players.position(first_id);
                    let second_position = ctx.tick_context.positions.players.position(second_id);

                    let midpoint = (first_position + second_position) * 0.5;

                    return Some(midpoint);
                }
            }
        }

        None
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        self.is_under_pressure(ctx)
    }

    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // Basic requirement: team must be in possession
        if !ctx.team().is_control_ball() {
            return false;
        }

        // Get player's mental attributes
        let vision = ctx.player.skills.mental.vision;
        let positioning = ctx.player.skills.mental.positioning;
        let teamwork = ctx.player.skills.mental.teamwork;
        let decisions = ctx.player.skills.mental.decisions;

        // Get physical attributes that affect ability to support
        let pace = ctx.player.skills.physical.pace;
        let stamina = ctx.player.skills.physical.stamina;
        let current_stamina = ctx.player.player_attributes.condition_percentage() as f32;

        // Calculate tactical intelligence - combination of mental attributes
        let tactical_intelligence = (vision + positioning + teamwork + decisions) / 40.0;

        // Players with lower tactical intelligence have stricter requirements
        let intelligence_threshold = if tactical_intelligence < 10.0 {
            // Low intelligence players only support when ball is very close
            50.0
        } else if tactical_intelligence < 14.0 {
            // Average intelligence players support when ball is moderately close
            120.0
        } else {
            // High intelligence players can read the game and support from further
            200.0
        };

        // Check if ball is within the player's tactical range
        let ball_distance = ctx.ball().distance();
        if ball_distance > intelligence_threshold {
            return false;
        }

        // Vision affects ability to see attacking opportunities
        let vision_range = vision * 15.0; // Better vision = see opportunities from further

        // Check if there are attacking teammates within vision range
        let attacking_teammates_nearby = ctx.players()
            .teammates()
            .nearby(vision_range)
            .filter(|teammate| {
                // Only consider forwards and attacking midfielders
                teammate.tactical_positions.is_forward() ||
                    (teammate.tactical_positions.is_midfielder() &&
                        self.is_in_attacking_position(ctx, teammate))
            })
            .count();

        // Players with good vision can spot opportunities even with fewer attacking players
        let min_attacking_players = if vision >= 16.0 {
            1 // Excellent vision - can create something from nothing
        } else if vision >= 12.0 {
            2 // Good vision - needs some support
        } else {
            3 // Poor vision - needs obvious attacking situation
        };

        if attacking_teammates_nearby < min_attacking_players {
            return false;
        }

        // Check stamina - tired players are less likely to make attacking runs
        let stamina_factor = (current_stamina / 100.0) * (stamina / 20.0);
        if stamina_factor < 0.6 {
            return false; // Too tired to support attack effectively
        }

        // Positioning skill affects understanding of when to support
        let positional_awareness = positioning / 20.0;

        // Check if player is in a good position to support (not too defensive)
        let field_length = ctx.context.field_size.width as f32;
        let player_field_position = match ctx.player.side {
            Some(PlayerSide::Left) => ctx.player.position.x / field_length,
            Some(PlayerSide::Right) => (field_length - ctx.player.position.x) / field_length,
            None => 0.5,
        };

        // Players with good positioning understand when they're too far back
        let min_field_position = if positional_awareness >= 0.8 {
            0.3 // Excellent positioning - can support from deeper
        } else if positional_awareness >= 0.6 {
            0.4 // Good positioning - needs to be in middle third
        } else {
            0.5 // Poor positioning - needs to be in attacking half
        };

        if player_field_position < min_field_position {
            return false;
        }

        // Check pace - slower players need to be closer to be effective
        let pace_factor = pace / 20.0;
        let effective_distance = if pace_factor >= 0.8 {
            200.0 // Fast players can support from further
        } else if pace_factor >= 0.6 {
            150.0 // Average pace players need to be closer
        } else {
            100.0 // Slow players need to be quite close
        };

        if ball_distance > effective_distance {
            return false;
        }

        // Teamwork affects willingness to make selfless runs
        let teamwork_factor = teamwork / 20.0;

        // Players with poor teamwork are more selfish and less likely to support
        if teamwork_factor < 0.5 {
            // Selfish players only support when they might get glory (very close to goal)
            return ctx.ball().distance_to_opponent_goal() < 150.0;
        }

        // Decision making affects timing of support runs
        let decision_quality = decisions / 20.0;

        // Poor decision makers might support at wrong times
        if decision_quality < 0.5 {
            // Check if this is actually a good time to support (not when defending)
            let opponents_in_defensive_third = ctx.players()
                .opponents()
                .all()
                .filter(|opponent| self.is_in_defensive_third(ctx, opponent))
                .count();

            // If many opponents in defensive third, poor decision makers might still go forward
            if opponents_in_defensive_third >= 3 {
                return false;
            }
        }

        // All checks passed - this player should support the attack
        true
    }

    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        let distance_from_start = ctx.player().distance_from_start_position();
        let team_in_possession = ctx.team().is_control_ball();

        distance_from_start > 200.0 && !team_in_possession
    }

    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(25.0)
    }

    fn is_in_attacking_position(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let field_length = ctx.context.field_size.width as f32;
        let attacking_third_start = match ctx.player.side {
            Some(PlayerSide::Left) => field_length * (2.0 / 3.0),
            Some(PlayerSide::Right) => field_length / 3.0,
            None => field_length * 0.5,
        };

        match ctx.player.side {
            Some(PlayerSide::Left) => teammate.position.x > attacking_third_start,
            Some(PlayerSide::Right) => teammate.position.x < attacking_third_start,
            None => false,
        }
    }

    fn is_in_defensive_third(&self, ctx: &StateProcessingContext, opponent: &MatchPlayerLite) -> bool {
        let field_length = ctx.context.field_size.width as f32;
        let defensive_third_end = match ctx.player.side {
            Some(PlayerSide::Left) => field_length / 3.0,
            Some(PlayerSide::Right) => field_length * (2.0 / 3.0),
            None => field_length * 0.5,
        };

        match ctx.player.side {
            Some(PlayerSide::Left) => opponent.position.x < defensive_third_end,
            Some(PlayerSide::Right) => opponent.position.x > defensive_third_end,
            None => false,
        }
    }
}