use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

const CREATING_SPACE_THRESHOLD: f32 = 150.0;
const OPPONENT_DISTANCE_THRESHOLD: f32 = 20.0;
const MAX_DISTANCE_FROM_START: f32 = 200.0; // Maximum distance from starting position
const RETURN_TO_POSITION_THRESHOLD: f32 = 250.0; // Distance to trigger return to position
const MAX_TIME_IN_STATE: u64 = 250; // Maximum time to stay in this state

#[derive(Default)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has the ball - immediate transition
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // Check if player has strayed too far from position
        if ctx.player().distance_from_start_position() > RETURN_TO_POSITION_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        // Check if team lost possession - switch to running for defensive positioning
        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // If the ball is close and moving toward player, try to intercept
        if ctx.ball().distance() < 200.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Add a time limit for staying in this state to prevent getting stuck
        if ctx.in_state_time > MAX_TIME_IN_STATE {
            if ctx.team().is_control_ball() {
                // If team has possession, go to assisting or running state
                if rand::random::<bool>() {
                    return Some(StateChangeResult::with_forward_state(ForwardState::Running));
                } else {
                    return Some(StateChangeResult::with_forward_state(ForwardState::Assisting));
                }
            } else {
                // If team doesn't have possession, go to running state
                return Some(StateChangeResult::with_forward_state(ForwardState::Running));
            }
        }

        // Check if the player has created enough space
        if self.has_created_space(ctx) {
            // If space is created, transition to the assisting state
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        }

        // Check if the player is too close to an opponent
        if self.should_dribble(ctx) {
            // If too close to an opponent, try to dribble away
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let target_position = self.calculate_space_creating_position(ctx);

        Some(
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 50.0,
            }
                .calculate(ctx.player)
                .velocity
                + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl ForwardCreatingSpaceState {
    fn has_created_space(&self, ctx: &StateProcessingContext) -> bool {
        // Check if there are no opponents within CREATING_SPACE_THRESHOLD
        let space_created = !ctx.players().opponents().exists(CREATING_SPACE_THRESHOLD);

        // Additional check: have we been in this state long enough?
        let minimum_time_in_state = 50;

        space_created && ctx.in_state_time > minimum_time_in_state
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        // Player should dribble if there's an opponent very close
        let close_opponent_threshold = 15.0;
        ctx.players().opponents().exists(close_opponent_threshold)
    }

    fn calculate_space_creating_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let player_side = ctx.player.side.unwrap_or(PlayerSide::Left);

        // Get ball position and team possession information
        let ball_position = ctx.tick_context.positions.ball.position;
        let team_in_possession = ctx.team().is_control_ball();

        // Find current ball holder on same team (if any)
        let ball_holder = if team_in_possession {
            ctx.players()
                .teammates()
                .all()
                .find(|t| ctx.ball().owner_id() == Some(t.id))
        } else {
            None
        };

        // If a teammate has the ball, create space away from them while maintaining attacking position
        if let Some(holder) = ball_holder {
            // Create space away from the ball holder but still in attacking position
            let holder_position = holder.position;
            let to_holder = holder_position - player_position;
            let direction_to_goal = ctx.player().opponent_goal_position() - player_position;

            // Calculate a perpendicular direction that tends toward the goal
            let perpendicular = Vector3::new(-to_holder.y, to_holder.x, 0.0).normalize();

            // Determine which perpendicular direction is more goal-oriented
            let dot_product = perpendicular.dot(&direction_to_goal);
            let goal_oriented_perpendicular = if dot_product >= 0.0 {
                perpendicular
            } else {
                -perpendicular
            };

            // Calculate position with a significant offset to create real space
            let target_position = player_position + goal_oriented_perpendicular * 60.0;

            // Add a slight forward bias toward goal (but not directly to it)
            let forward_bias = direction_to_goal.normalize() * 15.0;
            let biased_position = target_position + forward_bias;

            // Ensure we're not moving too far from starting position
            let distance_from_start = (biased_position - ctx.player.start_position).magnitude();
            if distance_from_start > MAX_DISTANCE_FROM_START {
                // Scale back the movement to stay within bounds
                let direction = (biased_position - ctx.player.start_position).normalize();
                let adjusted_position = ctx.player.start_position + direction * MAX_DISTANCE_FROM_START;

                // Ensure we stay in bounds
                return Vector3::new(
                    adjusted_position.x.clamp(20.0, field_width - 20.0),
                    adjusted_position.y.clamp(20.0, field_height - 20.0),
                    0.0,
                );
            }

            // Ensure we stay in bounds
            return Vector3::new(
                biased_position.x.clamp(20.0, field_width - 20.0),
                biased_position.y.clamp(20.0, field_height - 20.0),
                0.0,
            );
        }

        // No teammate has the ball - move to a strategic attacking position

        // Get attacking third position based on team side
        let attacking_third_x = if player_side == PlayerSide::Left {
            // For left side team, move toward the right (opponent's) side but not all the way
            field_width * 0.7
        } else {
            // For right side team, move toward the left (opponent's) side but not all the way
            field_width * 0.3
        };

        // Define zones where forward might create space - variety of positions
        let potential_zones = [
            Vector3::new(attacking_third_x, field_height * 0.3, 0.0),  // Wide left
            Vector3::new(attacking_third_x, field_height * 0.5, 0.0),  // Center
            Vector3::new(attacking_third_x, field_height * 0.7, 0.0),  // Wide right
            Vector3::new(attacking_third_x - 50.0, field_height * 0.4, 0.0),  // Deeper left
            Vector3::new(attacking_third_x - 50.0, field_height * 0.6, 0.0),  // Deeper right
        ];

        // Find position with the fewest opponents nearby
        let best_position = potential_zones.iter()
            .min_by_key(|&&pos| {
                // Count opponents within 30 units
                let opponent_count = ctx.players().opponents().all()
                    .filter(|o| (o.position - pos).magnitude() < 30.0)
                    .count();

                // Add slight preference for positions closer to goal line
                let attacking_preference = if player_side == PlayerSide::Left {
                    ((field_width - pos.x) / 50.0) as usize
                } else {
                    (pos.x / 50.0) as usize
                };

                opponent_count + attacking_preference
            })
            .copied()
            .unwrap_or(Vector3::new(attacking_third_x, field_height * 0.5, 0.0));

        // Add some randomization to prevent predictability and stickiness
        let jitter_x = (rand::random::<f32>() - 0.5) * 15.0;
        let jitter_y = (rand::random::<f32>() - 0.5) * 15.0;
        let jittered_position = Vector3::new(
            best_position.x + jitter_x,
            best_position.y + jitter_y,
            0.0
        );

        // Limit movement from starting position if needed
        let distance_from_start = (jittered_position - ctx.player.start_position).magnitude();
        if distance_from_start > MAX_DISTANCE_FROM_START {
            let direction = (jittered_position - ctx.player.start_position).normalize();
            let bounded_position = ctx.player.start_position + direction * MAX_DISTANCE_FROM_START;

            // Final boundary check
            return Vector3::new(
                bounded_position.x.clamp(20.0, field_width - 20.0),
                bounded_position.y.clamp(20.0, field_height - 20.0),
                0.0,
            );
        }

        // Final boundary check
        Vector3::new(
            jittered_position.x.clamp(20.0, field_width - 20.0),
            jittered_position.y.clamp(20.0, field_height - 20.0),
            0.0,
        )
    }
}