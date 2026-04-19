use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const PRESSURE_DISTANCE_THRESHOLD: f32 = 20.0; // Maximum distance from the goal to be considered under pressure

#[derive(Default, Clone)]
pub struct GoalkeeperPressureState {}

impl StateProcessingHandler for GoalkeeperPressureState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing
            ));
        }

        // Loose ball nearby — claim it instead of staying in dead-end pressure state
        if !ctx.ball().is_owned() && ctx.ball().distance() < 10.0 {
            let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
            if ball_speed < 5.0 {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Catching,
                ));
            }
        }

        if ctx.player().distance_from_start_position() > PRESSURE_DISTANCE_THRESHOLD {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        None
    }


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the start position (goal) using steering behavior
        let to_start_position = SteeringBehavior::Seek {
            target: ctx.player.start_position,
        }
        .calculate(ctx.player)
        .velocity;

        Some(to_start_position)
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Under pressure requires high intensity as goalkeeper moves back quickly
        GoalkeeperCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
