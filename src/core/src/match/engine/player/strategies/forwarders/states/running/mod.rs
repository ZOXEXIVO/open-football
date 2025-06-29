use crate::IntegerUtils;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerDistanceFromStartPosition, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 300.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 10.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

const MAX_LONG_SHOOTING_DISTANCE: f32 = 500.0; // Maximum distance to attempt a shot
const MIN_LONG_SHOOTING_DISTANCE: f32 = 300.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

#[derive(Default)]
pub struct ForwardRunningState {}

impl StateProcessingHandler for ForwardRunningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Handle cases when player has the ball
        if ctx.player.has_ball(ctx) {
            if self.has_clear_shot(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            if self.in_long_distance_shooting_range(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            if self.in_shooting_range(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Shooting,
                ));
            }

            if self.should_pass(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }

            if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            }

            // If none of the above conditions are met, forward should continue running with the ball
            return None;
        }
        // Handle cases when player doesn't have the ball
        else {
            // Check for interception opportunities
            if self.should_intercept(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Intercepting,
                ));
            }

            // Check if the player should press
            if self.should_press(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ));
            }

            // Check if the player should take a more supportive role
            if ctx.team().is_control_ball() && !self.is_in_good_attacking_position(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::CreatingSpace,
                ));
            }

            // Return to position if too far from starting position and team doesn't have ball
            if !ctx.team().is_control_ball() &&
                ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Big {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Returning,
                ));
            }

            // Randomly switch to other states to prevent getting stuck
            if ctx.in_state_time > 200 && rand::random::<f32>() < 0.1 {
                if ctx.team().is_control_ball() {
                    return Some(StateChangeResult::with_forward_state(
                        ForwardState::CreatingSpace,
                    ));
                }
                // } else {
                //     return Some(StateChangeResult::with_forward_state(
                //         ForwardState::Walking,
                //     ));
                // }
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If the player should follow waypoints, do so
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

        // Look for space between opponents when player has the ball
        if ctx.player.has_ball(ctx) {
            if let Some(target_position) = self.find_space_between_opponents(ctx) {
                Some(
                    SteeringBehavior::Arrive {
                        target: target_position,
                        slowing_distance: 10.0,
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity(),
                )
            } else {
                // Move toward goal if no space found
                Some(
                    SteeringBehavior::Arrive {
                        target: ctx.player().opponent_goal_position(),
                        slowing_distance: 100.0,
                    }
                        .calculate(ctx.player)
                        .velocity + ctx.player().separation_velocity(),
                )
            }
        }
        // Team has possession but this player doesn't have the ball
        else if ctx.team().is_control_ball() {
            // Calculate a tactical run position based on where ball holder is
            let tactical_run_position = self.calculate_tactical_run_position(ctx);

            return Some(
                SteeringBehavior::Arrive {
                    target: tactical_run_position,
                    slowing_distance: 30.0,
                }
                    .calculate(ctx.player)
                    .velocity + ctx.player().separation_velocity(),
            );
        }
        // Team doesn't have possession
        else {
            // Move with more purpose in defensive positioning
            let defensive_position = self.calculate_defensive_position(ctx);

            return Some(
                SteeringBehavior::Arrive {
                    target: defensive_position,
                    slowing_distance: 50.0,
                }
                    .calculate(ctx.player)
                    .velocity + ctx.player().separation_velocity(),
            );
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardRunningState {
    fn in_long_distance_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        (MIN_LONG_SHOOTING_DISTANCE..=MAX_LONG_SHOOTING_DISTANCE).contains(&distance_to_goal) &&
            // Add shooting skill check for long-range shots
            ctx.player.skills.technical.long_shots > 15.0
    }

    fn in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        (MIN_SHOOTING_DISTANCE..=MAX_SHOOTING_DISTANCE).contains(&distance_to_goal)
    }

    fn has_clear_shot(&self, ctx: &StateProcessingContext) -> bool {
        if ctx.ball().distance_to_opponent_goal() < MAX_SHOOTING_DISTANCE {
            return ctx.player().has_clear_shot();
        }
        false
    }

    fn should_intercept(&self, ctx: &StateProcessingContext) -> bool {
        // Don't try to intercept if ball is already owned
        if ctx.ball().is_owned() {
            return false;
        }

        // Check if ball is moving toward player at reasonable distance
        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return true;
        }

        // Check if ball is very close
        if ctx.ball().distance() < 50.0 {
            return true;
        }

        false
    }

    fn should_press(&self, ctx: &StateProcessingContext) -> bool {
        // More aggressive pressing for forwards - willing to press from further away
        let pressing_distance = 150.0;

        // Should press if opponent has ball and is within pressing range
        !ctx.team().is_control_ball() &&
            ctx.ball().distance() < pressing_distance &&
            // Make sure player isn't already too far from ideal position
            ctx.player().position_to_distance() != PlayerDistanceFromStartPosition::Big
    }

    fn should_pass(&self, ctx: &StateProcessingContext) -> bool {
        // Should pass if under pressure from opponents
        if ctx.players().opponents().exists(15.0) {
            return true;
        }

        // Should pass if player has good vision and can see teammates in better position
        let vision_threshold = 14.0;
        if ctx.player.skills.mental.vision >= vision_threshold {
            return self.find_open_teammate(ctx).is_some();
        }

        false
    }

    fn find_open_teammate(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        // Look for teammates in better attacking positions
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;

        // Calculate forward distance threshold based on team side
        let forward_distance = match ctx.player.side {
            Some(PlayerSide::Left) => player_position.x + 20.0,
            Some(PlayerSide::Right) => player_position.x - 20.0,
            None => return None,
        };

        // Find teammates who are more advanced and not closely marked
        let open_teammates: Vec<MatchPlayerLite> = ctx
            .players()
            .teammates()
            .nearby(200.0)
            .filter(|teammate| {
                // Check if teammate is more advanced
                let is_more_advanced = match ctx.player.side {
                    Some(PlayerSide::Left) => teammate.position.x > forward_distance,
                    Some(PlayerSide::Right) => teammate.position.x < forward_distance,
                    None => false,
                };

                // Check if teammate is open (not closely marked)
                let is_open = !ctx
                    .players()
                    .opponents()
                    .all()
                    .any(|opponent| (opponent.position - teammate.position).magnitude() < 10.0);

                is_more_advanced && is_open && ctx.player().has_clear_pass(teammate.id)
            })
            .collect();

        if open_teammates.is_empty() {
            None
        } else {
            // Find the teammate in best position
            open_teammates.into_iter()
                .min_by(|a, b| {
                    let a_goal_dist = (a.position - ctx.player().opponent_goal_position()).magnitude();
                    let b_goal_dist = (b.position - ctx.player().opponent_goal_position()).magnitude();
                    a_goal_dist.partial_cmp(&b_goal_dist).unwrap()
                })
        }
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
        let ball_position = ctx.tick_context.positions.ball.position;
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