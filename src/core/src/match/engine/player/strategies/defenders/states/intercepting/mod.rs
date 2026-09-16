use crate::r#match::common_states::LooseBallChase;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{ActivityIntensity, DefenderCondition};
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
        if let Some(carrier) = ctx.players().opponents().with_ball().next() {
            if TackleEngagement::should_commit(ctx, carrier.distance(ctx)) {
                return Some(StateChangeResult::with_defender_state(
                    DefenderState::Tackling,
                ));
            }
        }

        // Only abandon interception if ball is moving away AND is far
        // Stationary balls (speed < 0.5) should not trigger this exit
        if ctx.ball().speed() > 0.5
            && (!ctx.ball().is_towards_player_with_angle(0.7) || ball_distance > 130.0)
        {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Returning,
            ));
        }

        if !self.can_reach_before_opponent(ctx) {
            // If not, transition to Pressing or HoldingLine state
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Pressing,
            ));
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
        LooseBallChase::wins_the_race(ctx, self.calculate_interception_point(ctx))
    }

    /// Where the ball can actually be met.
    ///
    /// # What this used to compute
    ///
    /// A ground ball was led by `distance / (pace + ball_speed)`, and
    /// `pace` is a 1-20 SKILL, not a speed — the divisor was dominated
    /// by an attribute in the wrong units, so the "time to intercept"
    /// came out around `distance / 15` whatever the ball was doing, and
    /// the lead it bought was a few tens of centimetres. The number was
    /// then fed to [`Self::can_reach_before_opponent`], which races it
    /// against opponents measured the same wrong way — the ratio hid the
    /// unit error, which is why it survived.
    ///
    /// One solve now, shared with the `TakeBall` states and with the
    /// steering above, so where a player is sent and where he is steered
    /// cannot disagree. See [`LooseBallChase::meeting_point`].
    fn calculate_interception_point(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let landing_position = ctx.tick_context.positions.ball.landing_position;

        // A ball still in the air has to come down before anyone can play
        // it, and where it lands is the meeting point.
        let is_aerial = (ball_position - landing_position).norm_squared() > 5.0 * 5.0;
        if is_aerial {
            return landing_position;
        }

        LooseBallChase::meeting_point(ctx, ball_position, ctx.tick_context.positions.ball.velocity)
    }
}
