use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const PRESSURE_DISTANCE_THRESHOLD: f32 = 20.0; // Maximum distance from the goal to be considered under pressure

#[derive(Default)]
pub struct GoalkeeperPressureState {}

impl StateProcessingHandler for GoalkeeperPressureState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing
            ));
        }

        if ctx.player().distance_from_start_position() > PRESSURE_DISTANCE_THRESHOLD {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
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
