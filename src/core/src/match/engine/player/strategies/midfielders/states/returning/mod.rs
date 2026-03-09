use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::strategies::common::players::MatchPlayerIteratorExt;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct MidfielderReturningState {}

impl StateProcessingHandler for MidfielderReturningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::TakeBall,
            ));
        }

        // CRITICAL: Tackle/press if an opponent has the ball nearby
        if let Some(opponent) = ctx.players().opponents().nearby(100.0).with_ball(ctx).next() {
            let opponent_distance = (opponent.position - ctx.player.position).magnitude();

            if opponent_distance < 40.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }
            if opponent_distance < 100.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }
        }

        if !ctx.team().is_control_ball() && ctx.ball().distance() < 250.0 && ctx.ball().is_towards_player_with_angle(0.8) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }

        // Guard attackers when ball is on our side
        if !ctx.team().is_control_ball() && ctx.ball().on_own_side() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Guarding,
            ));
        }

        // If team has possession, switch to supporting instead of returning home
        if ctx.team().is_control_ball() && ctx.ball().distance() < 300.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        // Transition to Running when close to position (don't walk, stay active)
        let distance_to_start = (ctx.player.position - ctx.player.start_position).magnitude();
        if distance_to_start < 80.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let dist_to_start = (ctx.player.position - ctx.player.start_position).magnitude();

        // Close enough — stop to prevent oscillation
        if dist_to_start < 8.0 {
            return Some(Vector3::zeros());
        }

        let arrive = SteeringBehavior::Arrive {
            target: ctx.player.start_position,
            slowing_distance: 50.0,
        }
        .calculate(ctx.player)
        .velocity;

        // Only add separation when far from target — prevents fighting near destination
        if dist_to_start > 30.0 {
            Some(arrive + ctx.player().separation_velocity() * 0.3)
        } else {
            Some(arrive)
        }
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Returning is moderate intensity - getting back to position
        MidfielderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
