use crate::r#match::common_states::LooseBallChase;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::midfielders::states::common::{ActivityIntensity, MidfielderCondition};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use std::cmp::Ordering;

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
            if !ctx.ball().is_owned() && ball_distance < 50.0 && ctx.ball().speed() < 3.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::TakeBall,
                ));
            }

            if ball_distance < 30.0 {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Tackling,
                ));
            }

            if !self.can_reach_before_opponent(ctx) {
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Pressing,
                ));
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
        // Calculate time for defender to reach interception point
        let interception_point = self.calculate_interception_point(ctx);
        let defender_distance = (interception_point - ctx.player.position).magnitude();
        let defender_speed = ctx.player.skills.physical.pace.max(0.1); // Avoid division by zero
        let defender_time = defender_distance / defender_speed;

        // Find the minimum time for any opponent to reach the interception point.
        //
        // The man CARRYING the ball is excluded: his distance to the
        // interception point is zero, so including him made this test
        // false for every owned ball and the whole state a pass-through
        // to Pressing. You do not race someone for a ball he already has
        // — you press him. Same fault, same fix, in all three copies of
        // this helper.
        let carrier = ctx.ball().owner_id();
        let opponent_time = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| Some(opponent.id) != carrier)
            .map(|opponent| {
                let player = ctx.player();
                let skills = player.skills(opponent.id);

                let opponent_speed = skills.physical.pace.max(0.1);
                let opponent_distance = (interception_point - opponent.position).magnitude();
                opponent_distance / opponent_speed
            })
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal))
            .unwrap_or(f32::MAX);

        // Return true if defender can reach before any opponent
        defender_time < opponent_time
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
