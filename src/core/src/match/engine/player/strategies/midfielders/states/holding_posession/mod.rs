use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler, SteeringBehavior};
use nalgebra::Vector3;

const MAX_SHOOTING_DISTANCE: f32 = 150.0; // Maximum distance to attempt a shot
const MIN_SHOOTING_DISTANCE: f32 = 20.0; // Minimum distance to attempt a shot (e.g., edge of penalty area)

#[derive(Default, Clone)]
pub struct MidfielderHoldingPossessionState {}

impl StateProcessingHandler for MidfielderHoldingPossessionState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if the midfielder has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ));
        }

        if self.is_in_shooting_range(ctx) && ctx.player().has_clear_shot() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        // Check if the midfielder is being pressured by opponents
        if self.is_under_pressure(ctx) {
            // Under pressure — pass immediately rather than dribble into trouble
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        // Don't hold possession for long — transition to Passing quickly
        // This ensures the PassEvaluator gets to run and find the best option
        if ctx.in_state_time > 5 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Passing,
            ));
        }

        None
    }


    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player().opponent_goal_position(),
                slowing_distance: 30.0,
            }
                .calculate(ctx.player)
                .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Holding possession is low intensity - controlled possession
        MidfielderCondition::new(ActivityIntensity::Low).process(ctx);
    }
}

impl MidfielderHoldingPossessionState {
    pub fn is_under_pressure(&self, ctx: &StateProcessingContext) -> bool {
        ctx.player().pressure().is_under_immediate_pressure()
    }

    fn is_in_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal <= MAX_SHOOTING_DISTANCE && distance_to_goal >= MIN_SHOOTING_DISTANCE
    }
}
