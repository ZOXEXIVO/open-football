use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;

const MAX_DIVE_TIME: f32 = 1.5; // Maximum time to stay in diving state (in seconds)
const BALL_CLAIM_DISTANCE: f32 = 2.0; // Distance to claim the ball after a dive (in meters)

#[derive(Default)]
pub struct GoalkeeperDivingState {}

impl StateProcessingHandler for GoalkeeperDivingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Passing,
            ));
        }

        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let ball_moving_away = ball_velocity.dot(&(ctx.player().opponent_goal_position() - ctx.player.position)) > 0.0;

        if ctx.ball().distance() > 100.0 && ball_moving_away {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if ctx.in_state_time as f32 / 100.0 > MAX_DIVE_TIME {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        if self.is_ball_caught(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::CaughtBall(ctx.player.id)),
            ));
        } else if self.is_ball_nearby(ctx) {
            return Some(StateChangeResult::with_event(Event::PlayerEvent(PlayerEvent::ClaimBall(ctx.player.id))));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let dive_direction = self.calculate_dive_direction(ctx);
        let dive_speed = self.calculate_dive_speed(ctx);

        Some(dive_direction * dive_speed)
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperDivingState {
    fn calculate_dive_direction(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        let future_ball_position = ball_position + ball_velocity * 0.5; // Predict ball position 0.5 seconds ahead

        let to_future_ball = future_ball_position - ctx.player.position;
        let mut dive_direction = to_future_ball.normalize();

        // Add some randomness to dive direction
        let random_angle = (rand::random::<f32>() - 0.5) * std::f32::consts::PI / 6.0; // Random angle between -30 and 30 degrees
        dive_direction = nalgebra::Rotation3::new(Vector3::z() * random_angle) * dive_direction;

        dive_direction
    }

    fn calculate_dive_speed(&self, ctx: &StateProcessingContext) -> f32 {
        let urgency = self.calculate_urgency(ctx);
        (ctx.player.skills.physical.acceleration + ctx.player.skills.physical.agility) * 0.2 * urgency
    }

    fn calculate_urgency(&self, ctx: &StateProcessingContext) -> f32 {
        let ball_position = ctx.tick_context.positions.ball.position;
        let ball_velocity = ctx.tick_context.positions.ball.velocity;

        let distance_to_goal = (ball_position - ctx.player().opponent_goal_position()).magnitude();
        let velocity_towards_goal = ball_velocity.dot(&(ctx.player().opponent_goal_position() - ball_position)).max(0.0);

        let urgency: f32 = (1.0 - distance_to_goal / 100.0) * (1.0 + velocity_towards_goal / 10.0);
        urgency.clamp(1.0, 2.0)
    }

    fn is_ball_caught(&self, ctx: &StateProcessingContext) -> bool {
        let ball_distance = ctx.ball().distance();
        let ball_speed = ctx.tick_context.positions.ball.velocity.magnitude();

        let catch_probability = ctx.player.skills.technical.first_touch / 20.0 * (1.0 - ball_speed / 20.0); // Adjust for ball speed

        let goalkeeper_height = 1.9 + (ctx.player.player_attributes.height as f32 - 180.0) / 100.0; // Height in meters
        let catch_distance = goalkeeper_height * 0.5; // Adjust for goalkeeper height

        ball_distance < catch_distance && rand::random::<f32>() < catch_probability
    }

    fn is_ball_nearby(&self, ctx: &StateProcessingContext) -> bool {
        let goalkeeper_height = 1.9 + (ctx.player.player_attributes.height as f32 - 180.0) / 100.0;
        let nearby_distance = BALL_CLAIM_DISTANCE + goalkeeper_height * 0.1; // Adjust for goalkeeper height

        ctx.ball().distance() < nearby_distance
    }
}