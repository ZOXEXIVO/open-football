use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const TAKEBALL_TIMEOUT: u64 = 200; // Give up after 200 ticks (~3.3 seconds)
const MAX_TAKEBALL_DISTANCE: f32 = 500.0; // Don't chase balls further than this - increased to ensure someone always goes

#[derive(Default)]
pub struct ForwardTakeBallState {}

impl StateProcessingHandler for ForwardTakeBallState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if ball is now owned by someone
        if ctx.ball().is_owned() {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        let ball_distance = ctx.ball().distance();
        let ball_position = ctx.tick_context.positions.ball.landing_position;

        // 1. Timeout check - give up after too long
        if ctx.in_state_time > TAKEBALL_TIMEOUT {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // 2. Distance check - ball too far away
        if ball_distance > MAX_TAKEBALL_DISTANCE {
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Running,
            ));
        }

        // 3. Check if opponent will reach ball first
        if let Some(closest_opponent) = ctx.players().opponents().all().min_by(|a, b| {
            let dist_a = (a.position - ball_position).magnitude();
            let dist_b = (b.position - ball_position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        }) {
            let opponent_distance = (closest_opponent.position - ball_position).magnitude();

            // If opponent is significantly closer (by 20+ units), give up and press
            if opponent_distance < ball_distance - 20.0 {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::Pressing,
                ));
            }
        }

        // 4. Check if teammate is closer to the ball
        if let Some(closest_teammate) = ctx.players().teammates().all().filter(|t| t.id != ctx.player.id).min_by(|a, b| {
            let dist_a = (a.position - ball_position).magnitude();
            let dist_b = (b.position - ball_position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        }) {
            let teammate_distance = (closest_teammate.position - ball_position).magnitude();

            // If teammate is significantly closer (by 15+ units), let them take it
            if teammate_distance < ball_distance - 15.0 {
                return Some(StateChangeResult::with_forward_state(
                    ForwardState::CreatingSpace,
                ));
            }
        }

        // Continue trying to take the ball
        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // If ball is aerial, target the landing position instead of current position
        let target = ctx.tick_context.positions.ball.landing_position;

        Some(
            SteeringBehavior::Arrive {
                target,
                slowing_distance: 0.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}
