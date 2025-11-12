use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardPressingState {}

impl StateProcessingHandler for ForwardPressingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        if ctx.ball().distance() < 30.0 {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Tackling,
            ));
        }

        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        } else if ctx.ball().on_own_side() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Standing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Only pursue if opponent has the ball
        if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
            // Pursue the ball (which is with the opponent)
            Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                    .calculate(ctx.player)
                    .velocity + ctx.player().separation_velocity(),
            )
        } else {
            // If no opponent has ball (teammate has it or it's loose), just maintain position
            Some(Vector3::zeros())
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pressing is high intensity - sustained running and pressure
        ForwardCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
