use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default)]
pub struct MidfielderReturningState {}

impl StateProcessingHandler for MidfielderReturningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Priority 0: Free ball nearby - go claim it
        if ctx.ball().should_take_ball_immediately() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
        }

        // CRITICAL: Tackle if an opponent has the ball nearby
        // Using new chaining syntax: nearby(30.0).with_ball(ctx)
        if let Some(opponent) = ctx.players().opponents().nearby(30.0).with_ball(ctx).next() {
            let opponent_distance = (opponent.position - ctx.player.position).magnitude();

            if opponent_distance < 30.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
        }

        if !ctx.team().is_control_ball() && ctx.ball().distance() < 250.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Use hysteresis: only transition to Walking when significantly close (< 80 units)
        // This prevents oscillation at the 100-unit boundary
        let distance_to_start = (ctx.player.position - ctx.player.start_position).magnitude();
        if distance_to_start < 80.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Walking,
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
                slowing_distance: 50.0,  // Increased from 10.0 to slow down earlier and prevent overshoot
            }
            .calculate(ctx.player)
            .velocity  + ctx.player().separation_velocity(),
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Returning is moderate intensity - getting back to position
        MidfielderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
