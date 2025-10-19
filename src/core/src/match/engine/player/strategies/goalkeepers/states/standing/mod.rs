use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, PlayerDistanceFromStartPosition, PlayerSide, StateChangeResult,
    StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const DANGER_ZONE_RADIUS: f32 = 30.0;
const OPTIMAL_DISTANCE_FROM_GOAL: f32 = 200.0;

#[derive(Default)]
pub struct GoalkeeperStandingState {}

impl StateProcessingHandler for GoalkeeperStandingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return if ctx.players().opponents().exists(DANGER_ZONE_RADIUS) {
                Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Passing,
                ))
            } else {
                Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Running,
                ))
            }
        }
        else {
            if ctx.ball().distance() < 150.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ComingOut
                ));
            }
        }

        if ctx.ball().on_own_side() && ctx.ball().distance() < 300.0 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Attentive,
            ));
        }

        match ctx.player().position_to_distance() {
            PlayerDistanceFromStartPosition::Small => {
                if ctx.ball().is_towards_player_with_angle(0.8) {
                    if ctx.ball().is_towards_player() {
                        return Some(StateChangeResult::with_goalkeeper_state(
                            GoalkeeperState::PreparingForSave,
                        ));
                    }
                }

                if self.is_opponent_in_danger_zone(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::UnderPressure,
                    ));
                }
            }
            PlayerDistanceFromStartPosition::Medium => {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::ComingOut,
                ));
            }
            PlayerDistanceFromStartPosition::Big => {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Walking,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        // For now, return None to indicate no state change
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}

impl GoalkeeperStandingState {
    fn is_opponent_in_danger_zone(&self, ctx: &StateProcessingContext) -> bool {
        if let Some(opponent_with_ball) = ctx.players().opponents().with_ball().next() {
            let opponent_distance = ctx
                .tick_context
                .distances
                .get(ctx.player.id, opponent_with_ball.id);

            return opponent_distance < DANGER_ZONE_RADIUS;
        }

        false
    }

    fn calculate_optimal_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let goal_center = ctx.ball().direction_to_own_goal();
        let ball_position = ctx.tick_context.positions.ball.position;

        // Calculate a position on the line between the ball and the center of the goal
        let to_ball = ball_position - goal_center;
        let optimal_position = goal_center + to_ball.normalize() * OPTIMAL_DISTANCE_FROM_GOAL;

        // Ensure the goalkeeper stays within the penalty area
        self.clamp_to_penalty_area(ctx, optimal_position)
    }

    fn clamp_to_penalty_area(
        &self,
        ctx: &StateProcessingContext,
        position: Vector3<f32>,
    ) -> Vector3<f32> {
        let penalty_area = ctx
            .context
            .penalty_area(ctx.player.side == Some(PlayerSide::Left));
        Vector3::new(
            position.x.clamp(penalty_area.min.x, penalty_area.max.x),
            position.y.clamp(penalty_area.min.y, penalty_area.max.y),
            0.0,
        )
    }
}
