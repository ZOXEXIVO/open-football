use crate::IntegerUtils;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 300.0;
const MIN_SHOOTING_DISTANCE: f32 = 10.0;
const MAX_LONG_SHOOTING_DISTANCE: f32 = 400.0;
const MIN_LONG_SHOOTING_DISTANCE: f32 = 200.0;
const OPTIMAL_SHOOTING_DISTANCE: f32 = 180.0;
const SPRINT_DURATION_THRESHOLD: u64 = 150; // Ticks before considering fatigue

#[derive(Default)]
pub struct ForwardRunningState {}

impl StateProcessingHandler for ForwardRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Handle cases when player has the ball
        if ctx.player.has_ball(ctx) {
            // Priority 1: Clear shooting opportunity
            if self.has_excellent_shooting_opportunity(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            // Priority 2: In shooting range with good angle
            if self.in_shooting_range(ctx) && self.has_good_shooting_angle(ctx) {
                // Consider shooting vs passing based on situation
                if self.should_shoot_over_pass(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Shooting,
                    ));
                }
            }

            // Priority 3: Under pressure - quick decision needed
            if self.is_under_immediate_pressure(ctx) {
                if self.should_pass_under_pressure(ctx) {
                    return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
                } else if self.can_dribble_out_of_pressure(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Dribbling,
                    ));
                }
            }

            // Priority 4: Evaluate best action based on game context
            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            }

            // Continue running with ball if no better option
            return None;
        }
        // Handle cases when player doesn't have the ball
        else {
            // Priority 1: Ball interception opportunity
            if self.can_intercept_ball(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Intercepting,
                ));
            }

            // Priority 2: Pressing opportunity
            if self.should_press(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ));
            }

            // Priority 3: Create space when team has possession
            if ctx.team().is_control_ball() {
                if self.should_create_space(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::CreatingSpace,
                    ));
                }

                // Make intelligent runs
                if self.should_make_run_in_behind(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::RunningInBehind,
                    ));
                }
            }

            // Priority 4: Defensive duties when needed
            if !ctx.team().is_control_ball() {
                if self.should_return_to_position(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Returning,
                    ));
                }

                if self.should_help_defend(ctx) {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Pressing,
                    ));
                }
            }

            // Consider fatigue and state duration
            if self.needs_recovery(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Standing,
                ));
            }

            // Prevent getting stuck in running state
            if ctx.in_state_time > 300 {
                if ctx.team().is_control_ball() {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::CreatingSpace,
                    ));
                } else {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::Standing,
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
        // Fatigue-aware velocity calculation
        let fatigue_factor = self.calculate_fatigue_factor(ctx);

        // If following waypoints (team tactical movement)
        if ctx.player.should_follow_waypoints(ctx) && !ctx.player.has_ball(ctx) {
            let waypoints = ctx.player.get_waypoints_as_vectors();

            if !waypoints.is_empty() {
                return Some(
                    SteeringBehavior::FollowPath {
                        waypoints,
                        current_waypoint: ctx.player.waypoint_manager.current_index,
                        path_offset: IntegerUtils::random(1, 10) as f32,
                    }
                        .calculate(ctx.player)
                        .velocity * fatigue_factor
                        + ctx.player().separation_velocity(),
                );
            }
        }

        // Movement with ball
        if ctx.player.has_ball(ctx) {
            return Some(self.calculate_ball_carrying_movement(ctx) * fatigue_factor);
        }
        // Team has possession but this player doesn't have the ball
        else if ctx.team().is_control_ball() {
            return Some(self.calculate_supporting_movement(ctx) * fatigue_factor);
        }
        // Team doesn't have possession
        else {
            return Some(self.calculate_defensive_movement(ctx) * fatigue_factor);
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardRunningState {
    /// Check for excellent shooting opportunity (clear sight, good distance, no pressure)
    fn has_excellent_shooting_opportunity(&self, ctx: &StateProcessingContext) -> bool {
        let distance = ctx.ball().distance_to_opponent_goal();

        // Optimal shooting range
        if distance > OPTIMAL_SHOOTING_DISTANCE - 50.0 && distance < OPTIMAL_SHOOTING_DISTANCE + 50.0 {
            // Check for clear shot and minimal pressure
            let clear_shot = ctx.player().has_clear_shot();
            let low_pressure = !ctx.players().opponents().exists(10.0);
            let good_angle = self.has_good_shooting_angle(ctx);

            return clear_shot && low_pressure && good_angle;
        }

        false
    }

    /// Improved shooting range check with skill consideration
    fn in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        let shooting_skill = ctx.player.skills.technical.finishing / 20.0;
        let long_shot_skill = ctx.player.skills.technical.long_shots / 20.0;

        // Adjust range based on skills
        let effective_max_distance = MAX_SHOOTING_DISTANCE * (0.8 + shooting_skill * 0.4);
        let effective_long_distance = MAX_LONG_SHOOTING_DISTANCE * (0.7 + long_shot_skill * 0.6);

        (distance_to_goal >= MIN_SHOOTING_DISTANCE && distance_to_goal <= effective_max_distance) ||
            (distance_to_goal >= MIN_LONG_SHOOTING_DISTANCE && distance_to_goal <= effective_long_distance && long_shot_skill > 0.7)
    }

    /// Check shooting angle quality
    fn has_good_shooting_angle(&self, ctx: &StateProcessingContext) -> bool {
        let goal_angle = ctx.player().goal_angle();
        // Good angle is less than 45 degrees off center
        goal_angle < std::f32::consts::PI / 4.0
    }

    /// Determine if should shoot instead of looking for pass
    fn should_shoot_over_pass(&self, ctx: &StateProcessingContext) -> bool {
        let distance = ctx.ball().distance_to_opponent_goal();
        let has_clear_shot = ctx.player().has_clear_shot();
        let confidence = ctx.player.skills.mental.composure / 20.0;
        let finishing = ctx.player.skills.technical.finishing / 20.0;

        // Very close to goal - almost always shoot
        if distance < 100.0 && has_clear_shot {
            return true;
        }

        // Good position and skills
        if distance < 200.0 && has_clear_shot && (confidence + finishing) / 2.0 > 0.6 {
            return true;
        }

        // Check if teammates are in worse positions
        let better_positioned_teammate = ctx.players().teammates().nearby(150.0)
            .any(|t| {
                let t_dist = (t.position - ctx.player().opponent_goal_position()).magnitude();
                t_dist < distance * 0.7 // Significantly closer
            });

        !better_positioned_teammate && has_clear_shot
    }

    /// Check if under immediate pressure
    fn is_under_immediate_pressure(&self, ctx: &StateProcessingContext) -> bool {
        let very_close_opponents = ctx.players().opponents().nearby(8.0).count();
        let close_opponents = ctx.players().opponents().nearby(15.0).count();

        very_close_opponents >= 1 || close_opponents >= 2
    }

    /// Determine if should pass when under pressure
    fn should_pass_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        // Check for available passing options
        let safe_pass_available = ctx.players().teammates().nearby(100.0)
            .any(|t| ctx.player().has_clear_pass(t.id));

        let composure = ctx.player.skills.mental.composure / 20.0;

        // Low composure players pass more under pressure
        safe_pass_available && (composure < 0.7 || ctx.players().opponents().nearby(5.0).count() >= 2)
    }

    /// Check if can dribble out of pressure
    fn can_dribble_out_of_pressure(&self, ctx: &StateProcessingContext) -> bool {
        let dribbling = ctx.player.skills.technical.dribbling / 20.0;
        let agility = ctx.player.skills.physical.agility / 20.0;
        let composure = ctx.player.skills.mental.composure / 20.0;

        let skill_factor = (dribbling * 0.5 + agility * 0.3 + composure * 0.2);

        // Check for escape route
        let has_space = self.find_dribbling_space(ctx).is_some();

        skill_factor > 0.6 && has_space
    }

    /// Find space to dribble into
    fn find_dribbling_space(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_direction = (ctx.player().opponent_goal_position() - player_pos).normalize();

        // Check multiple angles for space
        let angles = [-45.0f32, -30.0, 0.0, 30.0, 45.0];

        for angle_deg in angles.iter() {
            let angle_rad = angle_deg.to_radians();
            let cos_a = angle_rad.cos();
            let sin_a = angle_rad.sin();

            // Rotate goal direction by angle
            let check_direction = Vector3::new(
                goal_direction.x * cos_a - goal_direction.y * sin_a,
                goal_direction.x * sin_a + goal_direction.y * cos_a,
                0.0
            );

            let check_position = player_pos + check_direction * 15.0;

            // Check if this direction is clear
            let opponents_in_path = ctx.players().opponents().all()
                .filter(|opp| {
                    let to_opp = opp.position - player_pos;
                    let dist = to_opp.magnitude();
                    let dot = to_opp.normalize().dot(&check_direction);

                    dist < 20.0 && dot > 0.7
                })
                .count();

            if opponents_in_path == 0 {
                return Some(check_position);
            }
        }

        None
    }

    /// Enhanced interception check
    fn can_intercept_ball(&self, ctx: &StateProcessingContext) -> bool {
        // Don't try to intercept if ball is already owned by teammate
        if ctx.ball().is_owned() {
            if let Some(owner_id) = ctx.ball().owner_id() {
                if let Some(owner) = ctx.context.players.by_id(owner_id) {
                    if owner.team_id == ctx.player.team_id {
                        return false;
                    }
                }
            }
        }

        let ball_distance = ctx.ball().distance();
        let ball_speed = ctx.ball().speed();

        // Static or slow-moving ball nearby
        if ball_distance < 30.0 && ball_speed < 2.0 {
            return true;
        }

        // Ball moving toward player
        if ball_distance < 150.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            // Calculate if player can reach interception point
            let player_speed = ctx.player.skills.physical.pace / 20.0 * 10.0;
            let time_to_reach = ball_distance / player_speed;
            let ball_travel_distance = ball_speed * time_to_reach;

            return ball_travel_distance < ball_distance * 1.5;
        }

        false
    }

    /// Improved pressing decision
    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // Don't press if team has possession
        if ctx.team().is_control_ball() {
            return false;
        }

        let ball_distance = ctx.ball().distance();
        let stamina_level = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;
        let work_rate = ctx.player.skills.mental.work_rate / 20.0;

        // Adjust pressing distance based on stamina and work rate
        let effective_press_distance = 150.0 * stamina_level * (0.5 + work_rate);

        // Check tactical instruction (high press vs low block)
        let high_press = ctx.team().tactics().is_high_pressing();

        if high_press {
            ball_distance < effective_press_distance * 1.3
        } else {
            // Only press in attacking third
            ball_distance < effective_press_distance && !ctx.ball().on_own_third()
        }
    }

    /// Determine if should create space
    fn should_create_space(&self, ctx: &StateProcessingContext) -> bool {
        // Don't create space if you're the closest to ball
        let closest_to_ball = !ctx.players().teammates().all()
            .any(|t| {
                let t_dist = (t.position - ctx.tick_context.positions.ball.position).magnitude();
                let p_dist = ctx.ball().distance();
                t_dist < p_dist * 0.9
            });

        if closest_to_ball {
            return false;
        }

        // Check if in good attacking position already
        if self.is_in_good_attacking_position(ctx) {
            return false;
        }

        // Create space if team has possession and player isn't needed for pressing
        true
    }

    /// Check if should make run in behind defense
    fn should_make_run_in_behind(&self, ctx: &StateProcessingContext) -> bool {
        // Check player attributes
        let pace = ctx.player.skills.physical.pace / 20.0;
        let off_ball = ctx.player.skills.mental.off_the_ball / 20.0;
        let stamina = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;

        if pace < 0.7 || off_ball < 0.6 || stamina < 0.4 {
            return false;
        }

        // Check if there's space behind defense
        let defensive_line = self.find_defensive_line(ctx);
        let space_behind = self.check_space_behind_defense(ctx, defensive_line);

        // Check if teammate can make the pass
        let capable_passer_has_ball = ctx.ball().owner_id()
            .and_then(|id| ctx.context.players.by_id(id))
            .map(|p| p.skills.technical.passing > 12.0)
            .unwrap_or(false);

        space_behind && capable_passer_has_ball && !ctx.player().on_own_side()
    }

    /// Find opponent defensive line position
    fn find_defensive_line(&self, ctx: &StateProcessingContext) -> f32 {
        let defenders: Vec<f32> = ctx.players().opponents().all()
            .filter(|p| p.tactical_positions.is_defender())
            .map(|p| match ctx.player.side {
                Some(PlayerSide::Left) => p.position.x,
                Some(PlayerSide::Right) => p.position.x,
                None => p.position.x,
            })
            .collect();

        if defenders.is_empty() {
            ctx.context.field_size.width as f32 / 2.0
        } else {
            // Return the position of the last defender
            match ctx.player.side {
                Some(PlayerSide::Left) => defenders.iter().fold(f32::MIN, |a, &b| a.max(b)),
                Some(PlayerSide::Right) => defenders.iter().fold(f32::MAX, |a, &b| a.min(b)),
                None => defenders.iter().sum::<f32>() / defenders.len() as f32,
            }
        }
    }

    /// Check if there's exploitable space behind defense
    fn check_space_behind_defense(&self, ctx: &StateProcessingContext, defensive_line: f32) -> bool {
        let player_x = ctx.player.position.x;

        match ctx.player.side {
            Some(PlayerSide::Left) => {
                // Space exists if defensive line is high and there's room behind
                defensive_line < ctx.context.field_size.width as f32 * 0.7 &&
                    player_x < defensive_line + 20.0
            },
            Some(PlayerSide::Right) => {
                defensive_line > ctx.context.field_size.width as f32 * 0.3 &&
                    player_x > defensive_line - 20.0
            },
            None => false,
        }
    }

    /// Determine if should return to defensive position
    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big
    }

    /// Check if forward should help defend
    fn should_help_defend(&self, ctx: &StateProcessingContext) -> bool {
        // Check game situation
        let losing_badly = ctx.team().is_loosing() && ctx.context.time.is_running_out();
        let work_rate = ctx.player.skills.mental.work_rate / 20.0;

        // High work rate forwards help more
        work_rate > 0.7 && losing_badly && ctx.ball().on_own_third()
    }

    /// Check if player needs recovery
    fn needs_recovery(&self, ctx: &StateProcessingContext) -> bool {
        let stamina = ctx.player.player_attributes.condition_percentage();
        let has_been_sprinting = ctx.in_state_time > SPRINT_DURATION_THRESHOLD;

        stamina < 60 && has_been_sprinting
    }

    /// Calculate fatigue factor for movement
    fn calculate_fatigue_factor(&self, ctx: &StateProcessingContext) -> f32 {
        let stamina = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;
        let time_in_state = ctx.in_state_time as f32;

        // Gradual fatigue over time
        let time_factor = (1.0 - (time_in_state / 500.0)).max(0.5);

        stamina * time_factor
    }

    /// Calculate movement when carrying the ball
    fn calculate_ball_carrying_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // First, look for optimal path to goal
        if let Some(target_position) = self.find_optimal_attacking_path(ctx) {
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 20.0,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity()
        } else {
            // Default to moving toward goal
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 100.0,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity()
        }
    }

    /// Find optimal path considering opponents and teammates
    fn find_optimal_attacking_path(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        // Look for gaps in defense
        if let Some(gap) = self.find_best_gap_in_defense(ctx) {
            return Some(gap);
        }

        // Try to move toward goal while avoiding opponents
        let to_goal = goal_pos - player_pos;
        let goal_direction = to_goal.normalize();

        // Check if direct path is clear
        if !ctx.players().opponents().nearby(30.0)
            .any(|opp| {
                let to_opp = opp.position - player_pos;
                let dot = to_opp.normalize().dot(&goal_direction);
                dot > 0.8 && to_opp.magnitude() < 40.0
            }) {
            return Some(player_pos + goal_direction * 50.0);
        }

        None
    }

    /// Find the best gap in opponent defense
    fn find_best_gap_in_defense(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_pos = ctx.player.position;
        let goal_pos = ctx.player().opponent_goal_position();

        let opponents: Vec<MatchPlayerLite> = ctx.players().opponents()
            .nearby(100.0)
            .filter(|opp| {
                // Only consider opponents between player and goal
                let to_goal = goal_pos - player_pos;
                let to_opp = opp.position - player_pos;
                to_goal.normalize().dot(&to_opp.normalize()) > 0.5
            })
            .collect();

        if opponents.len() < 2 {
            return None;
        }

        // Find largest gap
        let mut best_gap = None;
        let mut best_gap_size = 0.0;

        for i in 0..opponents.len() {
            for j in i+1..opponents.len() {
                let gap_center = (opponents[i].position + opponents[j].position) * 0.5;
                let gap_size = (opponents[i].position - opponents[j].position).magnitude();

                if gap_size > best_gap_size && gap_size > 20.0 {
                    best_gap_size = gap_size;
                    best_gap = Some(gap_center);
                }
            }
        }

        best_gap
    }

    /// Calculate supporting movement when team has ball
    fn calculate_supporting_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // Find ball holder
        let ball_holder = ctx.ball().owner_id()
            .and_then(|id| ctx.context.players.by_id(id))
            .filter(|p| p.team_id == ctx.player.team_id);

        if let Some(holder) = ball_holder {
            // Make intelligent supporting run
            let support_position = self.calculate_support_run_position(ctx, holder.position);

            SteeringBehavior::Arrive {
                target: support_position,
                slowing_distance: 30.0,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity()
        } else {
            // Move toward ball if no clear holder
            SteeringBehavior::Arrive {
                target: ctx.tick_context.positions.ball.position,
                slowing_distance: 50.0,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity()
        }
    }

    /// Calculate intelligent support run position
    fn calculate_support_run_position(&self, ctx: &StateProcessingContext, holder_pos: Vector3<f32>) -> Vector3<f32> {
        let player_pos = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine player's role based on position
        let is_central = (player_pos.y - field_height / 2.0).abs() < field_height * 0.2;
        let is_wide = !is_central;

        if is_wide {
            // Wide players make runs down the flanks
            self.calculate_wide_support_position(ctx, holder_pos)
        } else {
            // Central players make runs through the middle
            self.calculate_central_support_position(ctx, holder_pos)
        }
    }

    /// Calculate wide support position
    fn calculate_wide_support_position(&self, ctx: &StateProcessingContext, holder_pos: Vector3<f32>) -> Vector3<f32> {
        let player_pos = ctx.player.position;
        let field_height = ctx.context.field_size.height as f32;

        // Stay wide and ahead of ball
        let target_y = if player_pos.y < field_height / 2.0 {
            field_height * 0.1 // Left wing
        } else {
            field_height * 0.9 // Right wing
        };

        let target_x = match ctx.player.side {
            Some(PlayerSide::Left) => holder_pos.x + 40.0,
            Some(PlayerSide::Right) => holder_pos.x - 40.0,
            None => holder_pos.x,
        };

        Vector3::new(target_x, target_y, 0.0)
    }

    /// Calculate central support position
    fn calculate_central_support_position(&self, ctx: &StateProcessingContext, holder_pos: Vector3<f32>) -> Vector3<f32> {
        let field_height = ctx.context.field_size.height as f32;

        // Move into space between defenders
        let target_x = match ctx.player.side {
            Some(PlayerSide::Left) => holder_pos.x + 50.0,
            Some(PlayerSide::Right) => holder_pos.x - 50.0,
            None => holder_pos.x,
        };

        // Vary position slightly to create unpredictability
        let y_variation = ((ctx.in_state_time as f32 * 0.1).sin() * 20.0);
        let target_y = field_height / 2.0 + y_variation;

        Vector3::new(target_x, target_y, 0.0)
    }

    /// Calculate defensive movement
    fn calculate_defensive_movement(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;

        // Forwards maintain higher defensive line
        let defensive_line = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.55,
            Some(PlayerSide::Right) => field_width * 0.45,
            None => field_width * 0.5,
        };

        // Stay compact with midfield
        let target_y = ctx.player.start_position.y;
        let target_x = defensive_line;

        SteeringBehavior::Arrive {
            target: Vector3::new(target_x, target_y, 0.0),
            slowing_distance: 40.0,
        }
            .calculate(ctx.player)
            .velocity
    }


    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Basic checks - if no teammates around, can't pass
        let players = ctx.players();
        let teammates = players.teammates();

        // Check if there are any teammates in reasonable passing range
        let nearby_teammates: Vec<MatchPlayerLite> = teammates.nearby(150.0).collect();
        if nearby_teammates.is_empty() {
            return false;
        }

        // Get player skills for decision making
        let vision_skill = ctx.player.skills.mental.vision / 20.0; // Normalize to 0-1
        let passing_skill = ctx.player.skills.technical.passing / 20.0;
        let decision_skill = ctx.player.skills.mental.decisions / 20.0;
        let selfishness = 1.0 - (ctx.player.skills.mental.teamwork / 20.0); // Higher teamwork = less selfish

        // 1. PRESSURE CHECK - Under immediate pressure from opponents
        let immediate_pressure_distance = 30.0;
        let pressing_opponents = ctx.players().opponents()
            .nearby(immediate_pressure_distance)
            .count();

        if pressing_opponents >= 2 {
            // Multiple opponents pressing - should pass unless very selfish
            if selfishness < 0.8 {
                return true;
            }
        }

        // 2. TACTICAL SITUATION - Check if in good position to continue or should pass
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();

        // If very close to goal, forwards tend to be more selfish
        if distance_to_goal < 30.0 {
            // Only pass if teammate is in much better position or under severe pressure
            let severe_pressure = pressing_opponents >= 3 ||
                ctx.players().opponents().nearby(30.0).count() >= 1;

            if !severe_pressure {
                // Check if any teammate is in clearly better scoring position
                let better_positioned_teammate = nearby_teammates.iter().any(|teammate| {
                    let teammate_goal_dist = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
                    let teammate_pressure = ctx.players().opponents()
                        .nearby(20.0)
                        .filter(|opp| (opp.position - teammate.position).magnitude() < 8.0)
                        .count();

                    // Teammate is significantly closer to goal and not heavily marked
                    teammate_goal_dist < distance_to_goal * 0.7 && teammate_pressure < 2
                });

                return better_positioned_teammate && selfishness < 0.6;
            } else {
                return true; // Under severe pressure near goal, must pass
            }
        }

        // 3. VISION AND AWARENESS CHECK - Good players see better passing opportunities
        if vision_skill > 0.7 {
            // High vision players can spot overlapping runs and through balls
            let overlapping_teammates = nearby_teammates.iter().filter(|teammate| {
                // Check if teammate is making a forward run
                let teammate_velocity = ctx.tick_context.positions.players.velocity(teammate.id);
                let is_moving_forward = teammate_velocity.magnitude() > 2.0;

                // Check if teammate is in advanced position
                let teammate_goal_dist = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
                let advancing_run = teammate_goal_dist < distance_to_goal * 1.1;

                is_moving_forward && advancing_run
            }).count();

            if overlapping_teammates > 0 && decision_skill > 0.6 {
                return true;
            }
        }

        // 4. SKILL-BASED DECISION MAKING
        if passing_skill > 0.8 && vision_skill > 0.7 {
            // Highly skilled passers look for creative opportunities
            let creative_pass_opportunity = nearby_teammates.iter().any(|teammate| {
                // Check if pass would break defensive lines
                let player_pos = ctx.player.position;
                let teammate_pos = teammate.position;

                // Count opponents between player and teammate
                let opponents_between = ctx.players().opponents().all()
                    .filter(|opp| {
                        // Simple check if opponent is roughly between player and teammate
                        let to_teammate = teammate_pos - player_pos;
                        let to_opponent = opp.position - player_pos;
                        let dot_product = to_teammate.normalize().dot(&to_opponent);

                        dot_product > 0.0 && dot_product < to_teammate.magnitude()
                    })
                    .count();

                // If passing through opponents and teammate has space
                opponents_between >= 1 && !self.is_teammate_heavily_marked(ctx, teammate)
            });

            if creative_pass_opportunity {
                return true;
            }
        }

        // 5. GAME SITUATION AWARENESS
        // Check time pressure and team needs
        let team_needs_goal = ctx.team().is_loosing() || ctx.context.time.is_running_out();

        if team_needs_goal {
            // When team needs goals, look for any decent passing opportunity
            let decent_opportunity = nearby_teammates.iter().any(|teammate| {
                let teammate_goal_dist = (teammate.position - ctx.player().opponent_goal_position()).magnitude();
                let in_better_position = teammate_goal_dist < distance_to_goal * 0.9;
                let not_heavily_marked = !self.is_teammate_heavily_marked(ctx, teammate);

                in_better_position && not_heavily_marked
            });

            if decent_opportunity && selfishness < 0.7 {
                return true;
            }
        }

        // 6. FATIGUE AND CONDITION CHECK
        let stamina_percentage = ctx.player.player_attributes.condition_percentage() as f32 / 100.0;
        if stamina_percentage < 0.6 {
            // Tired players are more likely to pass to conserve energy
            let safe_pass_available = nearby_teammates.iter().any(|teammate| {
                ctx.player().has_clear_pass(teammate.id) &&
                    !self.is_teammate_heavily_marked(ctx, teammate)
            });

            if safe_pass_available {
                return true;
            }
        }

        // 7. DEFAULT BEHAVIOR - Continue dribbling if no strong reason to pass
        // But occasionally pass even without perfect reason (adds realism)
        if decision_skill > 0.8 && rand::random::<f32>() < 0.1 {
            // Very good decision makers sometimes make unexpected but good passes
            return nearby_teammates.iter().any(|teammate| {
                ctx.player().has_clear_pass(teammate.id)
            });
        }

        false
    }

    fn is_teammate_heavily_marked(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let marking_distance = 8.0;
        let markers = ctx.players().opponents().nearby(marking_distance).count();

        markers >= 2 || (markers >= 1 && ctx.players().opponents().nearby(3.0).count() > 0)    
    }
    
    fn find_space_between_opponents(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let opponent_goal = ctx.player().opponent_goal_position();
        let players = ctx.players();
        let opponents = players.opponents();

        // Get opponents between player and goal
        let player_position = ctx.player.position;
        let opponents_in_path: Vec<(u32, f32)> = opponents
            .nearby_raw(200.0)
            .filter(|(opp_id, _)| {
                let opp_pos = ctx.tick_context.positions.players.position(*opp_id);
                let to_goal = opponent_goal - player_position;
                let to_opp = opp_pos - player_position;

                // Check if opponent is roughly between player and goal
                to_goal.normalize().dot(&to_opp.normalize()) > 0.7
            })
            .collect();

        if opponents_in_path.len() < 2 {
            // Not enough opponents to find meaningful gap
            return None;
        }

        // Find the best gap between opponents
        let mut best_gap_position = None;
        let mut best_gap_score = 0.0;

        for i in 0..opponents_in_path.len() {
            for j in i+1..opponents_in_path.len() {
                let first_id = opponents_in_path[i].0;
                let second_id = opponents_in_path[j].0;

                let first_position = ctx.tick_context.positions.players.position(first_id);
                let second_position = ctx.tick_context.positions.players.position(second_id);

                // Calculate midpoint between opponents
                let midpoint = (first_position + second_position) * 0.5;

                // Calculate gap width
                let gap_width = (first_position - second_position).magnitude();

                // Calculate alignment with goal direction
                let to_goal = opponent_goal - player_position;
                let to_gap = midpoint - player_position;
                let alignment = to_goal.normalize().dot(&to_gap.normalize());

                // Calculate final gap score
                let gap_score = gap_width * alignment;

                if gap_score > best_gap_score && gap_width > 15.0 {
                    best_gap_score = gap_score;
                    best_gap_position = Some(midpoint);
                }
            }
        }

        best_gap_position
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Check player's dribbling skill
        let dribbling_skill = ctx.player.skills.technical.dribbling;

        // Check if there's space to dribble
        let has_space = !ctx.players().opponents().exists(15.0);

        // Forwards with good dribbling should try to dribble more often when they have space
        dribbling_skill > 15.0 && has_space
    }

    fn is_in_good_attacking_position(&self, ctx: &StateProcessingContext) -> bool {
        // Check if player is well-positioned in attacking third
        let field_width = ctx.context.field_size.width as f32;
        let attacking_third_start = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.65,
            Some(PlayerSide::Right) => field_width * 0.35,
            None => field_width * 0.5,
        };

        match ctx.player.side {
            Some(PlayerSide::Left) => ctx.player.position.x > attacking_third_start,
            Some(PlayerSide::Right) => ctx.player.position.x < attacking_third_start,
            None => false,
        }
    }

    // Calculate tactical run position for better support when team has possession
    fn calculate_tactical_run_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Find teammate with the ball
        let ball_holder = ctx.players()
            .teammates()
            .all()
            .find(|t| ctx.ball().owner_id() == Some(t.id));

        if let Some(holder) = ball_holder {
            // Calculate position based on ball holder's position
            let holder_position = holder.position;

            // Make runs beyond the ball holder
            let forward_position = match ctx.player.side {
                Some(PlayerSide::Left) => Vector3::new(
                    holder_position.x + 80.0,
                    // Vary Y-position based on player's current position
                    if player_position.y < field_height / 2.0 {
                        holder_position.y - 40.0  // Make run to left side
                    } else {
                        holder_position.y + 40.0  // Make run to right side
                    },
                    0.0
                ),
                Some(PlayerSide::Right) => Vector3::new(
                    holder_position.x - 80.0,
                    if player_position.y < field_height / 2.0 {
                        holder_position.y - 40.0  // Make run to left side
                    } else {
                        holder_position.y + 40.0  // Make run to right side
                    },
                    0.0
                ),
                None => Vector3::new(
                    holder_position.x,
                    holder_position.y + 30.0,
                    0.0
                ),
            };

            // Ensure position is within field boundaries
            return Vector3::new(
                forward_position.x.clamp(20.0, field_width - 20.0),
                forward_position.y.clamp(20.0, field_height - 20.0),
                0.0
            );
        }

        // Default to moving toward opponent's goal if no teammate has the ball
        let goal_direction = (ctx.player().opponent_goal_position() - player_position).normalize();
        player_position + goal_direction * 50.0
    }

    // Calculate defensive position when team doesn't have possession
    fn calculate_defensive_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;

        // Forwards generally stay higher up the pitch
        let forward_line = match ctx.player.side {
            Some(PlayerSide::Left) => field_width * 0.6,
            Some(PlayerSide::Right) => field_width * 0.4,
            None => field_width * 0.5,
        };

        // Use player's start position Y-coordinate for width positioning
        let target_y = ctx.player.start_position.y;

        Vector3::new(forward_line, target_y, 0.0)
    }
}