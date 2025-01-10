use nalgebra::Vector3;

use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler};

const MAX_DEFENSIVE_LINE_DEVIATION: f32 = 100.0;
const BALL_PROXIMITY_THRESHOLD: f32 = 100.0;
const MARKING_DISTANCE_THRESHOLD: f32 = 30.0;

#[derive(Default)]
pub struct DefenderHoldingLineState {}

impl StateProcessingHandler for DefenderHoldingLineState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Calculate the defensive line position
        let defensive_line_position = self.calculate_defensive_line_position(ctx);

        // 2. Calculate the distance from the defender to the defensive line
        let distance_from_line = (ctx.player.position.y - defensive_line_position).abs();

        // 3. If the defender is too far from the defensive line, switch to Running state
        if distance_from_line > MAX_DEFENSIVE_LINE_DEVIATION {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        if ctx.ball().distance() < 250.0 && ctx.ball().is_towards_player_with_angle(0.9) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Intercepting
            ));
        }

        if ctx.ball().distance() < BALL_PROXIMITY_THRESHOLD {
            let opponent_nearby = self.is_opponent_nearby(ctx);
            return Some(StateChangeResult::with_defender_state(if opponent_nearby {
                DefenderState::Marking
            } else {
                DefenderState::Intercepting
            }));
        }

        // 6. Check if we should set up an offside trap
        if self.should_set_offside_trap(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::OffsideTrap,
            ));
        }

        // 7. Remain in HoldingLine state
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        // For now, return None to indicate no state change
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let defensive_line_position = self.calculate_defensive_line_position(ctx);
        let current_position = ctx.player.position;
        let target_position = Vector3::new(current_position.x, defensive_line_position, current_position.z);

        // Calculate the distance between the current position and the target position
        let distance = (target_position - current_position).magnitude();

        // Define a minimum distance threshold to prevent oscillation
        const MIN_DISTANCE_THRESHOLD: f32 = 150.0;

        if distance > MIN_DISTANCE_THRESHOLD {
            // Calculate the direction from the current position to the target position
            let direction = (target_position - current_position).normalize();

            // Define a smooth speed factor based on the distance
            let speed_factor = (distance / MAX_DEFENSIVE_LINE_DEVIATION).clamp(0.1, 1.0);

            // Calculate the velocity based on the direction and speed factor
            let velocity = direction * speed_factor * ctx.player.skills.physical.pace;

            Some(velocity)
        } else {
            // If the distance is below the threshold, return zero velocity to prevent oscillation
            Some(Vector3::zeros())
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl DefenderHoldingLineState {
    /// Calculates the defensive line position based on team tactics and defender positions.
    fn calculate_defensive_line_position(&self, ctx: &StateProcessingContext) -> f32 {
        let defenders: Vec<MatchPlayerLite> = ctx.players().teammates().defenders().collect();

        // Calculate the average y-position of defenders to determine the defensive line
        let sum_y_positions: f32 = defenders.iter().map(|p| p.position.x).sum();
        sum_y_positions / defenders.len() as f32
    }

    /// Checks if an opponent player is nearby within the MARKING_DISTANCE_THRESHOLD.
    fn is_opponent_nearby(&self, ctx: &StateProcessingContext) -> bool {
        ctx.players().opponents().exists(MARKING_DISTANCE_THRESHOLD)
    }

    /// Determines if the team should set up an offside trap.
    fn should_set_offside_trap(&self, ctx: &StateProcessingContext) -> bool {
        // Check if opponents are positioned ahead of the defensive line
        let defensive_line_position = self.calculate_defensive_line_position(ctx);

        let opponents_ahead = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                if ctx.player.side == Some(PlayerSide::Left) {
                    opponent.position.x < defensive_line_position
                } else {
                    opponent.position.x > defensive_line_position
                }
            })
            .count();

        // If multiple opponents are ahead, consider setting up an offside trap
        opponents_ahead >= 2
    }
}
