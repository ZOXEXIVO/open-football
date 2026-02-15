use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const RESTING_STAMINA_THRESHOLD: u32 = 60; // Minimum stamina to transition out of resting state

#[derive(Default)]
pub struct GoalkeeperRestingState {}

impl StateProcessingHandler for GoalkeeperRestingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Emergency response: fast ball heading towards player
        if ctx.ball().is_towards_player_with_angle(0.7)
            && ctx.ball().distance() < 100.0
            && ctx.tick_context.positions.ball.velocity.norm() > 8.0
        {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        if ctx.player.player_attributes.condition_percentage() >= RESTING_STAMINA_THRESHOLD {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Resting state has the best recovery for goalkeepers in any state
        GoalkeeperCondition::new(ActivityIntensity::Recovery).process(ctx);
    }
}
