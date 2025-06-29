use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperPreparingForSaveState {}

impl StateProcessingHandler for GoalkeeperPreparingForSaveState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Passing,
            ));
        } else {
            if ctx.player().distance_from_start_position() > 150.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Attentive
                ));
            }

            if ctx.ball().distance() < 50.0 {
                if self.is_ball_catchable(ctx) {
                    return Some(StateChangeResult::with_goalkeeper_state(
                        GoalkeeperState::Catching,
                    ));
                }
            }

            if ctx.team().is_control_ball() {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Attentive
                ));
            }

            if self.should_dive(ctx) {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Diving,
                ));
            }
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperPreparingForSaveState {
    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        if ctx.ball().distance() > 10.0 {
            return false;
        }

        // Check if the ball is moving fast towards the goal
        ball_velocity.dot(&(ctx.ball().direction_to_own_goal() - ctx.player.position)) > 0.0
    }

    fn is_ball_catchable(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let goalkeeper_reach = ctx.player.skills.physical.jumping * 0.5 + 2.0; // Adjust as needed

        ball_distance < goalkeeper_reach && ball_speed < 10.0
    }
}
