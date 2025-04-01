use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const CREATING_SPACE_THRESHOLD: f32 = 150.0;
const OPPONENT_DISTANCE_THRESHOLD: f32 = 20.0;
const MAX_DISTANCE_FROM_START: f32 = 200.0; // Maximum distance from starting position
const RETURN_TO_POSITION_THRESHOLD: f32 = 250.0; // Distance to trigger return to position

#[derive(Default)]
pub struct ForwardCreatingSpaceState {}

impl StateProcessingHandler for ForwardCreatingSpaceState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if player has strayed too far from position
        if ctx.player().distance_from_start_position() > RETURN_TO_POSITION_THRESHOLD {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        if !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ctx.ball().distance() < 200.0 && !ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
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
        // Get an intelligent position to move to that creates space
        let target_position = self.calculate_space_creating_position(ctx);

        Some(
            SteeringBehavior::Arrive {
                target: target_position,
                slowing_distance: 50.0,
            }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity()
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No specific conditions to process
    }
}

impl ForwardCreatingSpaceState {
    fn has_created_space(&self, ctx: &StateProcessingContext) -> bool {
        !ctx.players().opponents().exists(CREATING_SPACE_THRESHOLD)
    }

    fn should_dribble(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player.has_ball(ctx) && ctx
            .players()
            .opponents()
            .exists(OPPONENT_DISTANCE_THRESHOLD)
    }

    /// Calculate a position that intelligently creates space based on the current game state
    fn calculate_space_creating_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let player_position = ctx.player.position;
        let field_width = ctx.context.field_size.width as f32;
        let field_height = ctx.context.field_size.height as f32;
        let player_side = ctx.player.side.unwrap_or(PlayerSide::Left);

        // Ensure we're moving in the correct direction based on player's team side
        let opponent_goal_position = if player_side == PlayerSide::Left {
            Vector3::new(field_width, field_height / 2.0, 0.0)
        } else {
            Vector3::new(0.0, field_height / 2.0, 0.0)
        };

        // Find current ball holder on same team (if any)
        let ball_holder = ctx.players().teammates().all()
            .find(|t| ctx.ball().owner_id() == Some(t.id));

        if let Some(holder) = ball_holder {
            // Create space away from the ball holder but still in attacking position
            let to_holder = holder.position - player_position;
            let perpendicular_direction = Vector3::new(-to_holder.y, to_holder.x, 0.0).normalize();

            // Choose side that's more toward the goal
            let perpendicular_pos1 = player_position + perpendicular_direction * 80.0;
            let perpendicular_pos2 = player_position - perpendicular_direction * 80.0;

            // Pick the position closer to the goal
            let dist1 = (perpendicular_pos1 - opponent_goal_position).magnitude();
            let dist2 = (perpendicular_pos2 - opponent_goal_position).magnitude();

            let target_position = if dist1 < dist2 {
                perpendicular_pos1
            } else {
                perpendicular_pos2
            };

            // Ensure we're not moving too far from starting position
            let distance_from_start = (target_position - ctx.player.start_position).magnitude();
            if distance_from_start > MAX_DISTANCE_FROM_START {
                // Scale back the movement to stay within bounds
                let direction = (target_position - ctx.player.start_position).normalize();
                return ctx.player.start_position + direction * MAX_DISTANCE_FROM_START;
            }

            // Ensure we stay in bounds
            let bounded_x = target_position.x.clamp(20.0, field_width - 20.0);
            let bounded_y = target_position.y.clamp(20.0, field_height - 20.0);

            return Vector3::new(bounded_x, bounded_y, 0.0);
        }

        // No teammate has the ball - move to an attacking position
        let attacking_third_x = if player_side == PlayerSide::Left {
            // For left side team, move toward the right (opponent's) side
            field_width * 0.75
        } else {
            // For right side team, move toward the left (opponent's) side
            field_width * 0.25
        };

        // Find a position that doesn't have many opponents nearby
        let potential_positions = [
            Vector3::new(attacking_third_x, field_height * 0.3, 0.0),
            Vector3::new(attacking_third_x, field_height * 0.5, 0.0),
            Vector3::new(attacking_third_x, field_height * 0.7, 0.0),
        ];

        // Choose position with fewest opponents nearby
        let best_position = potential_positions.iter()
            .min_by_key(|&&pos| {
                // Count opponents within 30 units
                ctx.players().opponents().all()
                    .filter(|o| (o.position - pos).magnitude() < 30.0)
                    .count()
            })
            .copied()
            .unwrap_or(Vector3::new(attacking_third_x, field_height * 0.5, 0.0));

        // Limit movement from starting position if needed
        let distance_from_start = (best_position - ctx.player.start_position).magnitude();
        if distance_from_start > MAX_DISTANCE_FROM_START {
            let direction = (best_position - ctx.player.start_position).normalize();
            return ctx.player.start_position + direction * MAX_DISTANCE_FROM_START;
        }

        best_position
    }
}