use crate::r#match::defenders::states::DefenderState;
use crate::r#match::defenders::states::common::{DefenderCondition, ActivityIntensity};
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;

const BLOCK_DISTANCE_THRESHOLD: f32 = 8.0; // Wide blocking zone - defender can lunge/stretch
const MAX_BLOCK_TIME: u64 = 500; // Don't stay in blocking state too long
const SHOT_SPEED_THRESHOLD: f32 = 0.3; // Ball speed indicating still in motion (shots max at ~2.0/tick)

#[derive(Default, Clone)]
pub struct DefenderBlockingState {}

impl StateProcessingHandler for DefenderBlockingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing
            ));
        }

        // Exit blocking if too much time has passed
        if ctx.in_state_time > MAX_BLOCK_TIME {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();
        let ball_distance = ctx.ball().distance();

        // Ball is very close — attempt to block/claim it
        if ball_distance < BLOCK_DISTANCE_THRESHOLD && !ctx.ball().is_owned() {
            let bravery = ctx.player.skills.mental.bravery / 20.0;
            let positioning = ctx.player.skills.mental.positioning / 20.0;
            let block_skill = bravery * 0.6 + positioning * 0.4;

            // Higher skill = higher block chance; proximity increases chance
            let proximity_bonus = 1.0 - (ball_distance / BLOCK_DISTANCE_THRESHOLD);
            let block_chance = block_skill * 0.5 + proximity_bonus * 0.3;

            if rand::random::<f32>() < block_chance {
                let mut result = StateChangeResult::with_defender_state(DefenderState::Standing);
                result.events.add_player_event(PlayerEvent::ClaimBall(ctx.player.id));
                return Some(result);
            }
        }

        // If ball has stopped, exit blocking
        if ball_speed < SHOT_SPEED_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // If ball is too far, exit blocking
        if ball_distance > BLOCK_DISTANCE_THRESHOLD * 4.0 {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        // If ball is not coming toward us anymore, exit
        if !ctx.ball().is_towards_player_with_angle(0.4) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_speed = ball_velocity.norm();

        // If ball is moving fast, try to intercept its path
        if ball_speed > SHOT_SPEED_THRESHOLD {
            // Predict where the ball will be
            let prediction_time = 0.3; // Look ahead 300ms
            let predicted_ball_pos = ball_position + ball_velocity * prediction_time;

            // Move toward predicted interception point
            let to_intercept = predicted_ball_pos - ctx.player.position;
            let intercept_distance = to_intercept.magnitude();

            if intercept_distance > 1.0 {
                // Use quick lateral movement to get in front of the ball
                let direction = to_intercept.normalize();
                let speed = ctx.player.skills.physical.pace * 0.8; // Quick but controlled
                return Some(direction * speed);
            }
        }

        // Position between ball and own goal
        let own_goal = ctx.ball().direction_to_own_goal();
        let ball_to_goal = (own_goal - ball_position).normalize();
        let blocking_position = ball_position + ball_to_goal * 2.0; // 2 meters in front of ball toward goal

        Some(
            SteeringBehavior::Arrive {
                target: blocking_position,
                slowing_distance: 3.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, ctx: ConditionContext) {
        // Blocking requires quick reactions - moderate intensity
        DefenderCondition::new(ActivityIntensity::Moderate).process(ctx);
    }
}
