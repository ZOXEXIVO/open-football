use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, Interception,
};
use crate::r#match::{
    ConditionContext, MATCH_TIME_MS, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

/// How close an assigned man has to be before a recovering defender
/// abandons the run to his slot and goes to him instead (~19 m). Same
/// figure `running` uses, so a duty means the same thing in every state
/// that holds it.
const MARK_RECOVERY_DISTANCE: f32 = 150.0;

#[derive(Default, Clone)]
pub struct DefenderReturningState {}

impl StateProcessingHandler for DefenderReturningState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Passing,
            ));
        }

        // Crisis override — abandon return journey and engage. Standing
        // will re-evaluate via the role block next tick.
        if ctx.player().defensive().is_defensive_crisis() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // Take ball only if best positioned — prevents swarming
        if ctx.ball().should_take_ball_immediately() && ctx.team().is_best_player_to_chase_ball() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // DUTY BEFORE POSITION — you recover goal-side of your MAN, not
        // to a slot.
        //
        // Only `Running` and `Guarding` read the plan; every recovery
        // state ignored it, and measured that is where the duties go:
        // **30% of all marking assignments were held by somebody in
        // Running / Returning / TrackingBack** — a duty nobody was acting
        // on — against 1% legitimately playing the ball. A defender
        // jogging back to his kickoff slot while the man he has been
        // given runs past him is the whole of "attackers in our third
        // with nobody within three metres".
        if let Some(man) = ctx.team().my_mark() {
            if (man.position - ctx.player.position).magnitude() < MARK_RECOVERY_DISTANCE {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Marking,
                ));
            }
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
        } else {
            if ctx.ball().distance() < 100.0 {
                if ctx.players().opponents().with_ball().next().is_some() {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::Tackling,
                    ));
                } else {
                    return Some(StateChangeResult::with_defender_state(
                        DefenderState::TakeBall,
                    ));
                }
            }

            if Interception::is_available(ctx) && ctx.ball().is_towards_player_with_angle(0.8) && ctx.ball().distance() < 200.0 {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Intercepting,
                ));
            }

            // Final ~1/15 of the match (~6 min full length) — the old
            // `- 180` was 180 ms, so the losing-team press never fired.
            if ctx.team().is_loosing()
                && ctx.context.total_match_time > MATCH_TIME_MS - MATCH_TIME_MS / 15
                && ctx.ball().distance() < 30.0
            {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Pressing,
                ));
            }
        }

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
        // Returning to position involves jogging back - moderate intensity
        DefenderCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
