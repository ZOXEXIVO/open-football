use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior, MATCH_TIME_MS,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct ForwardReturningState {}

impl StateProcessingHandler for ForwardReturningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ctx.team().is_control_ball(){
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }
        
        if !ctx.team().is_control_ball() && ctx.ball().distance() < 200.0 {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        if !ctx.team().is_control_ball() && ctx.ball().distance() < 100.0 {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Tackling,
            ));
        }

        // Stay in returning state until very close to start position
        if ctx.player().distance_from_start_position() < 2.0 {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Standing,
            ));
        }

        // Intercept if ball coming towards player and is closer than before
        if !ctx.team().is_control_ball() && ctx.ball().is_towards_player_with_angle(0.9) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Intercepting,
            ));
        }

        // Transition to Pressing late in the game only if ball is close as well
        if ctx.team().is_loosing()
            && ctx.context.total_match_time > (MATCH_TIME_MS - 180)
            && ctx.ball().distance() < 30.0
        {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Pressing,
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
        // Returning is moderate intensity - getting back to position
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
