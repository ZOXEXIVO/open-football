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
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Passing,
                ));
            }

            if self.should_dribble(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Dribbling,
                ));
            }

            if self.is_under_pressure(ctx) {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Passing,
                ));
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
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Passing,
                ));
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
                        .velocity + ctx.player().separation_velocity(),
                );
            }
        }

        // Handle ball possession differently
        if ctx.player.has_ball(ctx) {
            // When player has the ball, move toward goal but be aware of opponents
            let goal_position = ctx.player().opponent_goal_position();

            // Check if there's heavy opposition in direct goal path
            if ctx.players().opponents().exists(25.0) {
                // Find space to move into rather than directly toward goal
                if let Some(target_position) = self.find_space_between_opponents(ctx) {
                    return Some(
                        SteeringBehavior::Arrive {
                            target: target_position,
                            slowing_distance: 10.0,
                        }
                            .calculate(ctx.player)
                            .velocity + ctx.player().separation_velocity(),
                    );
                }
            }

            // Default movement with ball - toward goal but with more caution
            return Some(
                SteeringBehavior::Arrive {
                    target: goal_position,
                    slowing_distance: 100.0,
                }
                    .calculate(ctx.player)
                    .velocity + ctx.player().separation_velocity(),
            );
        }
        // Team has possession, but this player doesn't have the ball
        else if ctx.team().is_control_ball() {
            // Find tactical positioning based on where the ball is
            let ball_position = ctx.tick_context.positions.ball.position;
            let tactical_position = self.calculate_tactical_position(ctx, ball_position);

            return Some(
                SteeringBehavior::Arrive {
                    target: tactical_position,
                    slowing_distance: 50.0,
                }
                    .calculate(ctx.player)
                    .velocity + ctx.player().separation_velocity(),
            );
        }
        // Defensive scenario - team doesn't have the ball
        else {
            // More measured movement - return to a defensive position
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
    fn calculate_tactical_position(&self, ctx: &StateProcessingContext, ball_position: Vector3<f32>) -> Vector3<f32> {
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
    fn is_ball_in_attacking_third(&self, ctx: &StateProcessingContext, ball_position: Vector3<f32>) -> bool {
        let field_width = ctx.context.field_size.width as f32;

        if ctx.player.side == Some(PlayerSide::Left) {
            ball_position.x > field_width * 0.66 // Final third for left team
        } else {
            ball_position.x < field_width * 0.33 // Final third for right team
        }
    }

    // Helper to check if ball is in middle third
    fn is_ball_in_middle_third(&self, ctx: &StateProcessingContext, ball_position: Vector3<f32>) -> bool {
        let field_width = ctx.context.field_size.width as f32;

        if ctx.player.side == Some(PlayerSide::Left) {
            ball_position.x > field_width * 0.33 && ball_position.x <= field_width * 0.66
        } else {
            ball_position.x >= field_width * 0.33 && ball_position.x < field_width * 0.66
        }
    }
}