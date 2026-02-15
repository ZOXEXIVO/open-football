use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct GoalkeeperReturningGoalState {}

impl StateProcessingHandler for GoalkeeperReturningGoalState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        // Loose ball very close — claim it instead of ignoring it
        if !ctx.ball().is_owned() && ctx.ball().distance() < 15.0 && ctx.ball().on_own_side() {
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            if ball_speed < 5.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Catching,
                ));
            }
        }

        if ctx.player().distance_from_start_position() < 50.0 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Walking,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player.start_position,
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Returning to goal requires high intensity as goalkeeper moves back quickly
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
