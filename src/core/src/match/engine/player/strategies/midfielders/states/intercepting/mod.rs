use crate::r#match::common_states::LooseBallChase;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// Aerial-contest band. Same values the defender's Intercepting state
/// uses, so the same dropping ball reads identically whichever role gets
/// to it first.
const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;

#[derive(Default, Clone)]
pub struct MidfielderInterceptingState {}

impl StateProcessingHandler for MidfielderInterceptingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        }

        // Aerial ball in reach — contest it in the air. Mirrors the
        // defender's Intercepting → Heading hand-off. Checked BEFORE the
        // possession branch because a lofted ball dropping into midfield
        // has to be attacked whoever nominally "has" the ball: this is
        // the second-ball contest, and without it midfielders were the
        // only outfield role that could never head the ball.
        // Own-corner deliveries are excluded: the discrete corner
        // contest owns that aerial (same gate as the Running entry).
        let ball_position = ctx.tick_context.positions.ball.position;
        if ball_position.z > HEADING_HEIGHT
            && ctx.ball().distance() < HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6)
            && !ctx.ball().is_team_attacking_corner()
        {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Heading,
            ));
        }

        // Team has ball — no need to intercept, transition out
        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Running,
            ));
        } else {
            let ball_distance = ctx.ball().distance();

            // Loose ball nearby — claim it directly instead of tackling thin air
            if !ctx.ball().is_owned()
                && ball_distance < 50.0
                && ctx.ball().speed() < 3.0
                && ctx.team().is_best_player_to_chase_ball()
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }

            if let Some(carrier) = ctx.players().opponents().with_ball().next()
                && TackleEngagement::should_commit(ctx, carrier.distance(ctx))
            {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }

            // Somebody has it: there is nothing left to intercept.
            if ctx.players().opponents().with_ball().next().is_some() {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
            }

            if !self.can_reach_before_opponent(ctx) {
                // Nobody to press unless somebody has it or the plan named him.
                let press = ctx.ball().is_owned() || TackleEngagement::is_nominated_presser(ctx);
                return Some(StateChangeResult::with_midfielder_state(if press {
                    MidfielderState::Pressing
                } else {
                    MidfielderState::Returning
                }));
            }
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // The state's own answer to where the ball can be met — which
        // until now it computed and then ignored. `velocity()` steered
        // at the ball's CURRENT position, while
        // `calculate_interception_point` was consulted only by
        // `can_reach_before_opponent`, so the state's steering and its
        // own idea of where the ball could be reached were unrelated.
        // One point, used for both.
        Some(
            SteeringBehavior::Intercept {
                target: ctx.tick_context.positions.ball.position,
                target_velocity: ctx.tick_context.positions.ball.velocity,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Reading a pass and going to it is explosive, not a cruise —
        // same tier as the back line's `Intercepting`, and for the same
        // reason (the tier is a speed cap).
        MidfielderCondition::with_velocity(ActivityIntensity::chase()).process(ctx);
    }
}

impl MidfielderInterceptingState {
    fn can_reach_before_opponent(&self, ctx: &StateProcessingContext) -> bool {
        LooseBallChase::can_contest(ctx)
    }
}
