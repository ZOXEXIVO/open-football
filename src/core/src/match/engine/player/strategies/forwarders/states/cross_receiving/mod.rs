use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::forwarders::states::common::{ActivityIntensity, ForwardCondition};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

/// A ground cross collected this close to goal is a cutback finish, not a
/// reception. Kept in step with `FINISHING_RANGE` in the finishing state.
const CUTBACK_FINISH_RANGE: f32 = 48.0;

#[derive(Default, Clone)]
pub struct ForwardCrossReceivingState {}

impl StateProcessingHandler for ForwardCrossReceivingState {
    fn process(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();

        if !ball_ops.is_towards_player_with_angle(0.8) || ctx.ball().distance() > 100.0 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ball_ops.distance() <= 10.0 {
            // Aerial ball — head it
            if ctx.tick_context.positions.ball.position.z >= 1.5 {
                return Some(StateChangeResult::with_forward_state(ForwardState::Heading));
            }

            // Ground ball — control it, and MOVE ON in the same result.
            //
            // This used to be an event-only result (`state: None`), and
            // `StateProcessingResult::merge_state_change` drops the events
            // of a result that carries no transition — so the receive
            // request never reached the dispatcher and the cross simply
            // rolled through the forward. Pairing the event with the
            // state the player is genuinely moving into fixes the drop at
            // this site without touching the global merge contract.
            //
            // A cutback arriving inside the box is a finishing chance;
            // anything further out is a normal reception.
            let next = if ball_ops.distance_to_opponent_goal() <= CUTBACK_FINISH_RANGE {
                ForwardState::Finishing
            } else {
                ForwardState::Running
            };
            return Some(StateChangeResult::with_forward_state_and_event(
                next,
                Event::PlayerEvent(PlayerEvent::RequestBallReceive(ctx.player.id)),
            ));
        }

        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        // For aerial balls, pursue the estimated landing position
        let target = if ball_position.z >= 1.5 && ball_velocity.z < 0.0 {
            // Estimate where the ball will land (simple ballistic: t = -vz/g,
            // then x += vx*t). `t` comes out in TICKS, which is what the
            // horizontal step below is denominated in — the bare `9.81` here
            // was a per-second figure and under-estimated the flight ~100×,
            // so the chaser aimed at the ball's current position and the
            // cross dropped behind him.
            let time_to_land =
                (-ball_velocity.z / crate::r#match::engine::ball::ball::GRAVITY_PER_TICK).max(0.0);
            Vector3::new(
                ball_position.x + ball_velocity.x * time_to_land,
                ball_position.y + ball_velocity.y * time_to_land,
                0.0,
            )
        } else {
            ball_position
        };

        Some(
            SteeringBehavior::Pursuit {
                target,
                target_velocity: ball_velocity,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Cross receiving is moderate intensity - positioning and timing
        ForwardCondition::with_velocity(ActivityIntensity::Moderate).process(ctx);
    }
}
