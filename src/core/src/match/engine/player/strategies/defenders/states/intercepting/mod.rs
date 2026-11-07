use crate::r#match::common_states::LooseBallChase;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{
    ActivityIntensity, DefenderCondition, Interception,
};
use crate::r#match::player::strategies::common::states::TackleEngagement;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const HEADING_HEIGHT: f32 = 1.5;
const HEADING_DISTANCE: f32 = 5.0;

#[derive(Default, Clone)]
pub struct DefenderInterceptingState {}

impl StateProcessingHandler for DefenderInterceptingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Running,
            ));
        }

        // Check if ball is aerial and at heading height
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_distance = ctx.ball().distance();

        if ball_position.z > HEADING_HEIGHT
            && ball_distance < HEADING_DISTANCE
            && ctx.ball().is_towards_player_with_angle(0.6)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Heading,
            ));
        }

        // Loose ball nearby — go claim it directly
        if !ctx.ball().is_owned()
            && ball_distance < 60.0
            && ctx.ball().speed() < 4.0
            && ctx.team().is_best_player_to_chase_ball()
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::TakeBall,
            ));
        }

        // A ball within 25u is not a reason to challenge anybody: this
        // asked BALL distance and never asked whether an opponent was
        // carrying it, so it handed a defender into a state that ejects
        // him again on the next line.
        if let Some(carrier) = ctx.players().opponents().with_ball().next()
            && TackleEngagement::should_commit(ctx, carrier.distance(ctx))
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Tackling,
            ));
        }

        // Somebody has it: there is nothing left to intercept.
        if ctx.players().opponents().with_ball().next().is_some() {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
        }

        // Only abandon interception if ball is moving away AND is far
        // Stationary balls (speed < 0.5) should not trigger this exit
        if ctx.ball().speed() > 0.5
            && (!ctx.ball().is_towards_player_with_angle(0.7)
                || ball_distance > Interception::GIVE_UP)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        if !self.can_reach_before_opponent(ctx) {
            // Nobody to press unless somebody has it or the plan named him.
            let press = ctx.ball().is_owned() || TackleEngagement::is_nominated_presser(ctx);
            return Some(StateChangeResult::with_defender_state(if press {
                DefenderState::Pressing
            } else {
                DefenderState::Returning
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
        // Reading a pass and going to it is explosive — the ball is
        // travelling and the window is a stride wide. `High` (0.78 of
        // top speed) is a cruise; see the note in
        // `DefenderPressingState` for why the tier is a speed cap.
        DefenderCondition::with_velocity(ActivityIntensity::chase()).process(ctx);
    }
}

impl DefenderInterceptingState {
    fn can_reach_before_opponent(&self, ctx: &StateProcessingContext) -> bool {
        LooseBallChase::can_contest(ctx)
    }
}
