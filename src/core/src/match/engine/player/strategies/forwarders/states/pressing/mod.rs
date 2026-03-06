use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// Maximum distance from start position before forward stops pressing and returns
const MAX_PRESS_DISTANCE_FROM_START: f32 = 200.0;
/// Ball must be within this range to keep pressing
const MAX_PRESS_BALL_DISTANCE: f32 = 120.0;

#[derive(Default, Clone)]
pub struct ForwardPressingState {}

impl StateProcessingHandler for ForwardPressingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // Back off during foul protection — don't crowd the free kick
        if ctx.ball().is_in_flight() && ctx.ball().is_owned() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // Loose ball nearby — go claim it directly instead of pressing thin air
        if !ctx.ball().is_owned() && ctx.ball().distance() < 50.0 && ctx.ball().speed() < 3.0 {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
            ));
        }

        if ctx.ball().distance() < 30.0 && ctx.ball().is_owned() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Tackling,
            ));
        }

        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Assisting,
            ));
        }

        // Ball moved away — stop pressing and return to position
        if ctx.ball().distance() > MAX_PRESS_BALL_DISTANCE {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        // Too far from start position — don't chase forever, return
        if ctx.player().distance_from_start_position() > MAX_PRESS_DISTANCE_FROM_START {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        if ctx.ball().on_own_side() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_distance = ctx.ball().distance();

        // Only pursue if opponent has the ball and it's within pressing range
        if let Some(_opponent) = ctx.players().opponents().with_ball().next() {
            if ball_distance < MAX_PRESS_BALL_DISTANCE {
                return Some(
                    SteeringBehavior::Pursuit {
                        target: ctx.tick_context.positions.ball.position,
                        target_velocity: ctx.tick_context.positions.ball.velocity,
                    }
                    .calculate(ctx.player)
                    .velocity + ctx.player().separation_velocity(),
                );
            }
        } else if !ctx.ball().is_owned() && ball_distance < 80.0 {
            // Loose ball nearby — pursue it
            return Some(
                SteeringBehavior::Pursuit {
                    target: ctx.tick_context.positions.ball.position,
                    target_velocity: ctx.tick_context.positions.ball.velocity,
                }
                .calculate(ctx.player)
                .velocity + ctx.player().separation_velocity(),
            );
        }

        // Ball too far or teammate has it — drift back toward start position
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player.start_position,
                slowing_distance: 30.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Pressing is high intensity - sustained running and pressure
        ForwardCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}
