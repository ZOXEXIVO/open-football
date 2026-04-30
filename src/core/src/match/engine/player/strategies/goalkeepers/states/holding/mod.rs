use crate::r#match::goalkeepers::states::common::{ActivityIntensity, GoalkeeperCondition};
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const MIN_HOLDING_DURATION: u64 = 25;
const MAX_HOLDING_DURATION: u64 = 60;

#[derive(Default, Clone)]
pub struct GoalkeeperHoldingState {}

impl StateProcessingHandler for GoalkeeperHoldingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // If for some reason we no longer have the ball, return to standing
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // After holding for a skill-based duration, transition to distribute the ball
        // Better decision-makers distribute faster
        let decision = ctx.player.skills.mental.decisions / 20.0;
        let holding_duration = MAX_HOLDING_DURATION
            - ((MAX_HOLDING_DURATION - MIN_HOLDING_DURATION) as f32 * decision) as u64;
        if ctx.in_state_time >= holding_duration {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Distributing,
            ));
        }

        // No other transitions - goalkeeper should continue holding the ball
        // until ready to distribute it, should not try to catch the same ball
        // they already possess
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Holding ball is a low intensity activity with minimal physical effort
        GoalkeeperCondition::new(ActivityIntensity::Low).process(ctx);
    }
}
