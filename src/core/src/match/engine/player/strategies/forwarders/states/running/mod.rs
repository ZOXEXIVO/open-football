use crate::IntegerUtils;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
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

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }
        } else {
            if self.should_intercept(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Intercepting,
                ));
            }

            if self.should_press(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ));
            }

            if self.should_support_attack(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Assisting,
                ));
            }

            if self.should_return_to_position(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Returning,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // First, check if following waypoints is appropriate
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
                    .velocity
                        + ctx.player().separation_velocity(),
                );
            }
        }

        // When player has the ball - smarter movement toward goal area, not directly to goal point
        if ctx.player.has_ball(ctx) {
            // Try to find space between opponents
            if let Some(target_position) = self.find_space_between_opponents(ctx) {
                return Some(
                    SteeringBehavior::Arrive {
                        target: target_position,
                        slowing_distance: 10.0,
                    }
                    .calculate(ctx.player)
                    .velocity
                        + ctx.player().separation_velocity(),
                );
            }

            // If no good space found, move toward a scoring position rather than directly to goal
            let goal_area_position = self.calculate_scoring_position(ctx);

            Some(
                SteeringBehavior::Arrive {
                    target: goal_area_position,
                    slowing_distance: 100.0,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity(),
            )
        }
        // Team has possession, but this player doesn't have the ball
        else if ctx.team().is_control_ball() {
            // Calculate a tactical run position
            let tactical_run_position = self.calculate_tactical_run_position(ctx);

            return Some(
                SteeringBehavior::Arrive {
                    target: tactical_run_position,
                    slowing_distance: 30.0,
                }
                .calculate(ctx.player)
                .velocity
                    + ctx.player().separation_velocity(),
            );
        }
        else {
            None
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardRunningState {
    fn calculate_tactical_run_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let ball_position = ctx.tick_context.positions.ball.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let player_side = ctx.player.side.unwrap_or(PlayerSide::Left);

        // Get direction of attack based on team side
        let attacking_direction = if player_side == PlayerSide::Left {
            Vector3::new(1.0, 0.0, 0.0)
        } else {
            Vector3::new(-1.0, 0.0, 0.0)
        };

        // Check if we're already in advanced position
        let is_advanced_position = if player_side == PlayerSide::Left {
            player_position.x > field_width * 0.7
        } else {
            player_position.x < field_width * 0.3
        };

        // If already in advanced position, make intelligent runs in the box
        if is_advanced_position {
            // Make runs into spaces in the penalty area
            let penalty_area_depth = 16.5 * 4.0; // Scaled penalty area depth
            let penalty_area_width = 40.3 * 3.0; // Scaled penalty area width

            // Determine penalty area coordinates
            let (penalty_area_start_x, penalty_area_y_center) = if player_side == PlayerSide::Left {
                (field_width - penalty_area_depth, field_height / 2.0)
            } else {
                (0.0, field_height / 2.0)
            };

            // Calculate potential run targets inside penalty area
            let run_targets = [
                // Near post run
                Vector3::new(
                    if player_side == PlayerSide::Left {
                        field_width - 60.0
                    } else {
                        60.0
                    },
                    penalty_area_y_center - 30.0,
                    0.0,
                ),
                // Far post run
                Vector3::new(
                    if player_side == PlayerSide::Left {
                        field_width - 60.0
                    } else {
                        60.0
                    },
                    penalty_area_y_center + 30.0,
                    0.0,
                ),
                // Cutback position
                Vector3::new(
                    if player_side == PlayerSide::Left {
                        field_width - 120.0
                    } else {
                        120.0
                    },
                    penalty_area_y_center,
                    0.0,
                ),
                // Penalty spot
                Vector3::new(
                    if player_side == PlayerSide::Left {
                        field_width - 90.0
                    } else {
                        90.0
                    },
                    penalty_area_y_center,
                    0.0,
                ),
            ];

            // Find run target with most space
            return run_targets
                .iter()
                .max_by(|&a, &b| {
                    let space_a = self.calculate_space_at_position(ctx, a);
                    let space_b = self.calculate_space_at_position(ctx, b);
                    space_a
                        .partial_cmp(&space_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .copied()
                .unwrap_or_else(|| {
                    // Default target if none found
                    Vector3::new(
                        if player_side == PlayerSide::Left {
                            field_width - 90.0
                        } else {
                            90.0
                        },
                        penalty_area_y_center,
                        0.0,
                    )
                });
        }

        // If teammate has the ball, make a supporting run
        if let Some(teammate_with_ball) = ctx.players().teammates().players_with_ball().next() {
            // Check if we should make an overlapping run
            let teammate_pos = teammate_with_ball.position;

            // Calculate if we're to the left or right of teammate
            let is_to_right = player_position.y > teammate_pos.y;

            // Make overlapping run
            return Vector3::new(
                teammate_pos.x + attacking_direction.x * 70.0,
                teammate_pos.y + (if is_to_right { 40.0 } else { -40.0 }),
                0.0,
            );
        }

        // Default to moving into a good attacking position ahead of the ball
        Vector3::new(
            ball_position.x + attacking_direction.x * 80.0,
            ball_position.y + (IntegerUtils::random(0, 100) as f32 - 50.0),
            0.0,
        )
    }

    // Calculate a scoring position rather than just the goal point
    fn calculate_scoring_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_side = ctx.player.side.unwrap_or(PlayerSide::Left);
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Determine goal coordinates
        let goal_x = if player_side == PlayerSide::Left {
            field_width
        } else {
            0.0
        };
        let goal_y = field_height / 2.0;

        // Create a position in front of goal that's better for shooting
        let shoot_distance = 50.0 + (rand::random::<f32>() * 20.0); // Slight randomness

        Vector3::new(
            if player_side == PlayerSide::Left {
                field_width - shoot_distance
            } else {
                shoot_distance
            },
            goal_y + (rand::random::<f32>() * 40.0 - 20.0), // Some variation in Y position
            0.0,
        )
    }

    // Helper function to calculate amount of space at a position
    fn calculate_space_at_position(
        &self,
        ctx: &StateProcessingContext,
        position: &Vector3<f32>,
    ) -> f32 {
        let space_radius = 15.0;
        let opponents_nearby = ctx
            .players()
            .opponents()
            .all()
            .filter(|o| (o.position - *position).magnitude() < space_radius)
            .count();

        space_radius - opponents_nearby as f32
    }

    // Find the best space between opponents
    fn find_space_between_opponents(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let player_position = ctx.player.position;
        let opponent_goal = ctx.player().opponent_goal_position();
        let players = ctx.players();
        let opponents = players.opponents();

        // Get up to 5 nearest opponents to analyze gaps
        let mut nearest_opponents: Vec<(u32, f32)> = opponents.nearby_raw(200.0).collect();
        nearest_opponents
            .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        if nearest_opponents.len() < 2 {
            return None;
        }

        // Find the best gap between opponents
        let mut best_gap_position = None;
        let mut best_gap_score = 0.0;

        // Try pairs of opponents to find gaps
        for i in 0..nearest_opponents.len() {
            for j in i + 1..nearest_opponents.len() {
                let first_id = nearest_opponents[i].0;
                let second_id = nearest_opponents[j].0;

                let first_position = ctx.tick_context.positions.players.position(first_id);
                let second_position = ctx.tick_context.positions.players.position(second_id);

                // Calculate midpoint
                let midpoint = (first_position + second_position) * 0.5;

                // Calculate gap width
                let gap_width = (first_position - second_position).magnitude();

                // Only consider reasonable sized gaps
                if gap_width < 10.0 {
                    continue;
                }

                // Score based on gap width, distance to goal, and alignment with goal
                let to_goal = opponent_goal - midpoint;
                let to_goal_normalized = to_goal.normalize();
                let gap_direction = (second_position - first_position).normalize();
                let alignment = gap_direction.dot(&to_goal_normalized).abs();

                let goal_distance = to_goal.magnitude();
                let gap_score = gap_width * alignment / (1.0 + goal_distance / 100.0);

                if gap_score > best_gap_score {
                    best_gap_position = Some(midpoint);
                    best_gap_score = gap_score;
                }
            }
        }

        best_gap_position
    }

    fn in_long_distance_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        (MIN_LONG_SHOOTING_DISTANCE..=MAX_LONG_SHOOTING_DISTANCE)
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

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        self.is_under_pressure(ctx)
    }

    fn should_support_attack(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the team is in possession and the player is in a good position to support the attack
        let team_in_possession = ctx.team().is_control_ball();
        let in_attacking_half = ctx.player.position.x > ctx.context.field_size.width as f32 / 2.0;

        team_in_possession && in_attacking_half && ctx.ball().distance() < 200.0
    }

    fn should_return_to_position(&self, ctx: &StateProcessingContext) -> bool {
        // Check if the player is far from their starting position and the team is not in possession
        let distance_from_start = ctx.player().distance_from_start_position();
        let team_in_possession = ctx.team().is_control_ball();

        distance_from_start > 20.0 && !team_in_possession
    }

    fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(25.0)
    }

    // Calculate a tactical attacking position based on ball position
    fn calculate_tactical_position(
        &self,
        ctx: &StateProcessingContext,
        ball_position: Vector3<f32>,
    ) -> Vector3<f32> {
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let goal_position = ctx.player().opponent_goal_position();

        // If ball is in attacking third, get into scoring position
        if self.is_ball_in_attacking_third(ctx, ball_position) {
            // Create a position in the box or near the goal
            let x_pos = if ctx.player.side == Some(PlayerSide::Left) {
                field_width * 0.85 // Near goal for left team
            } else {
                field_width * 0.15 // Near goal for right team
            };

            // Vary vertical position to create space
            let player_offset = ((ctx.player.id % 10) as f32) * 0.05;
            let y_pos = field_height * (0.4 + player_offset); // Different for each player

            return Vector3::new(x_pos, y_pos, 0.0);
        }
        // Ball in middle third - support attack
        else if self.is_ball_in_middle_third(ctx, ball_position) {
            // Position slightly ahead of ball but not offside
            let x_offset = if ctx.player.side == Some(PlayerSide::Left) {
                50.0 // Forward of ball for left team
            } else {
                -50.0 // Forward of ball for right team
            };

            // Vary position based on player to create width
            let y_offset = ((ctx.player.id % 3) as f32 - 1.0) * 80.0;

            return Vector3::new(ball_position.x + x_offset, ball_position.y + y_offset, 0.0);
        }
        // Ball in defensive third - provide outlet
        else {
            // Position in middle third as outlet for clearance
            let x_pos = if ctx.player.side == Some(PlayerSide::Left) {
                field_width * 0.6 // Middle-attacking third for left team
            } else {
                field_width * 0.4 // Middle-attacking third for right team
            };

            // Vary vertical position
            let y_pos = field_height * (0.3 + ((ctx.player.id % 5) as f32) * 0.1);

            return Vector3::new(x_pos, y_pos, 0.0);
        }
    }

    // Calculate a defensive position when team doesn't have the ball
    fn calculate_defensive_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        // For forwards, defensive position is usually in middle third
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;

        // Position in middle third based on side
        let x_pos = if ctx.player.side == Some(PlayerSide::Left) {
            field_width * 0.4 // Middle third for left team
        } else {
            field_width * 0.6 // Middle third for right team
        };

        // Vary position based on player id to spread out
        let player_offset = ((ctx.player.id % 10) as f32) * 0.05;
        let y_pos = field_height * (0.35 + player_offset);

        Vector3::new(x_pos, y_pos, 0.0)
    }

    // Helper to check if ball is in attacking third
    fn is_ball_in_attacking_third(
        &self,
        ctx: &StateProcessingContext,
        ball_position: Vector3<f32>,
    ) -> bool {
        let field_width = ctx.context.field_size.width as f32;

        if ctx.player.side == Some(PlayerSide::Left) {
            ball_position.x > field_width * 0.66 // Final third for left team
        } else {
            ball_position.x < field_width * 0.33 // Final third for right team
        }
    }

    // Helper to check if ball is in middle third
    fn is_ball_in_middle_third(
        &self,
        ctx: &StateProcessingContext,
        ball_position: Vector3<f32>,
    ) -> bool {
        let field_width = ctx.context.field_size.width as f32;

        if ctx.player.side == Some(PlayerSide::Left) {
            ball_position.x > field_width * 0.33 && ball_position.x <= field_width * 0.66
        } else {
            ball_position.x >= field_width * 0.33 && ball_position.x < field_width * 0.66
        }
    }
}
