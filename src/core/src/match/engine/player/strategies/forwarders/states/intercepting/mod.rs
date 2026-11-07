use crate::r#match::common_states::LooseBallChase;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{
    ActivityIntensity, ForwardCondition, InterceptionRange,
};
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

#[derive(Default, Clone)]
pub struct ForwardInterceptingState {}

impl StateProcessingHandler for ForwardInterceptingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ctx.team().is_control_ball() {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        let ball_distance = ctx.ball().distance();

        // Give up beyond the OUTER band. `Returning` commits at
        // `InterceptionRange::COMMIT`, so the give-up distance has to sit
        // outside it or the overlap makes the two states a two-cycle —
        // see `InterceptionRange` for the measurement.
        if ball_distance > InterceptionRange::GIVE_UP {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Returning,
            ));
        }

        // Loose ball nearby — claim it directly
        if !ctx.ball().is_owned()
            && ball_distance < 50.0
            && ctx.ball().speed() < 3.0
            && ctx.team().is_best_player_to_chase_ball()
        {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::TakeBall,
            ));
        }

        if let Some(carrier) = ctx.players().opponents().with_ball().next()
            && TackleEngagement::should_commit(ctx, carrier.distance(ctx))
        {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Tackling,
            ));
        }

        // 2. Check if the player can reach the interception point before any opponent
        if !self.can_reach_before_opponent(ctx) {
            // Nobody to press unless somebody has it or the plan named him.
            let press = ctx.ball().is_owned() || TackleEngagement::is_nominated_presser(ctx);
            return Some(StateChangeResult::with_forward_state(if press {
                ForwardState::Pressing
            } else {
                ForwardState::Returning
            }));
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
        // `High`, matching the defender and midfielder interception states,
        // because this is a speed cap and not a mood: `Moderate` held the
        // forward to 0.52 of top speed while `velocity()` above is a
        // full-blooded `Pursuit` onto a moving ball. Reading the pass is
        // the cheap part; getting there is a sprint, and the two players
        // who might beat him to it are both allowed 0.78.
        ForwardCondition::with_velocity(ActivityIntensity::High).process(ctx);
    }
}

impl ForwardInterceptingState {
    fn can_reach_before_opponent(&self, ctx: &StateProcessingContext) -> bool {
        LooseBallChase::can_contest(ctx)
    }
}
