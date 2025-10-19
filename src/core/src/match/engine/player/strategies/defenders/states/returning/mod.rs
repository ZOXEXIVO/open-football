use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior, MATCH_HALF_TIME_MS,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct DefenderReturningState {}

impl StateProcessingHandler for DefenderReturningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Passing,
            ));
        }

        if ctx.player().distance_from_start_position() < 10.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }
        
        if ctx.team().is_control_ball() {
            if ctx.player().distance_from_start_position() < 5.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Standing,
                ));
            }
        }
        else {
            if ctx.ball().distance() < 100.0{
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }

            if ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 200.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting
                ));
            }

            if ctx.team().is_loosing()
                && ctx.context.time.time > (MATCH_HALF_TIME_MS - 180)
                && ctx.ball().distance() < 30.0
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
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
            SteeringBehavior::Arrive {
                target: ctx.player.start_position,
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}
