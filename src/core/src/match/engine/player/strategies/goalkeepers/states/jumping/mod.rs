use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use crate::r#match::player::events::PlayerEvent;
use nalgebra::Vector3;
use std::sync::LazyLock;

static _GOALKEEPER_JUMPING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_jumping_data.json")));

// Jump parameters
const JUMP_DURATION: u64 = 30; // Duration of jump animation in ticks
const JUMP_HEIGHT: f32 = 2.5; // Maximum jump height
const MIN_DIVING_DISTANCE: f32 = 1.0; // Minimum distance to dive
const MAX_DIVING_DISTANCE: f32 = 5.0; // Maximum distance to dive

#[derive(Default)]
pub struct GoalkeeperJumpingState {}

impl StateProcessingHandler for GoalkeeperJumpingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Check if jump duration is complete
        if ctx.in_state_time >= JUMP_DURATION {
            // After jump, transition to appropriate state
            if ctx.player.has_ball(ctx) {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::HoldingBall
                ));
            } else {
                return Some(StateChangeResult::with_goalkeeper_state(
                    GoalkeeperState::Standing
                ));
            }
        }

        // During jump, check if we can catch the ball
        if self.can_catch_ball(ctx) {
            let mut result = StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Catching
            );

            // Add catch attempt event
            result.events.add_player_event(PlayerEvent::RequestBallReceive(ctx.player.id));
            return Some(result);
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Calculate base jump vector
        let jump_vector = self.calculate_jump_vector(ctx);

        // Add diving motion if needed
        let diving_vector = if self.should_dive(ctx) {
            self.calculate_diving_vector(ctx)
        } else {
            Vector3::zeros()
        };

        // Calculate vertical component based on jump phase
        let vertical_component = self.calculate_vertical_motion(ctx);

        // Combine all motion components
        let combined_velocity = jump_vector + diving_vector + Vector3::new(0.0, 0.0, vertical_component);

        // Scale based on goalkeeper's jumping and agility attributes
        let attribute_scaling = (ctx.player.skills.physical.jumping as f32 +
            ctx.player.skills.physical.agility as f32) / 40.0;

        Some(combined_velocity * attribute_scaling)
    }

    fn process_conditions(&self, _ctx: ConditionContext) {

    }
}

impl GoalkeeperJumpingState {
    /// Check if the goalkeeper can reach and catch the ball
    fn can_catch_ball(&self, ctx: &StateProcessingContext) -> bool {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let distance = (ball_pos - keeper_pos).magnitude();

        // Calculate reach based on goalkeeper height and jumping ability
        let max_reach = JUMP_HEIGHT * (ctx.player.skills.physical.jumping as f32 / 20.0);

        // Check if ball is within reach considering vertical position
        let vertical_reach = (ball_pos.z - keeper_pos.z).abs() <= max_reach;
        let horizontal_reach = distance <= MAX_DIVING_DISTANCE;

        vertical_reach && horizontal_reach
    }

    /// Calculate the base jump vector towards the ball
    fn calculate_jump_vector(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let to_ball = ball_pos - keeper_pos;

        if to_ball.magnitude() > 0.0 {
            to_ball.normalize() * ctx.player.skills.physical.acceleration
        } else {
            Vector3::zeros()
        }
    }

    /// Determine if the goalkeeper should dive
    fn should_dive(&self, ctx: &StateProcessingContext) -> bool {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let distance = (ball_pos - keeper_pos).magnitude();

        // Check if the ball is at a distance that requires diving
        if distance < MIN_DIVING_DISTANCE || distance > MAX_DIVING_DISTANCE {
            return false;
        }

        // Check if the ball is moving towards goal
        let ball_velocity = ctx.tick_context.positions.ball.velocity;
        let to_goal = ctx.ball().direction_to_own_goal() - ball_pos;

        ball_velocity.dot(&to_goal) > 0.0
    }

    /// Calculate the diving motion vector
    fn calculate_diving_vector(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_pos = ctx.tick_context.positions.ball.position;
        let keeper_pos = ctx.player.position;
        let to_ball = ball_pos - keeper_pos;

        if to_ball.magnitude() > 0.0 {
            // Calculate diving direction considering goalkeeper's diving ability
            let diving_direction = to_ball.normalize();
            let diving_power = ctx.player.skills.physical.jumping as f32 / 20.0;

            diving_direction * diving_power * 2.0
        } else {
            Vector3::zeros()
        }
    }

    /// Calculate vertical motion based on jump phase
    fn calculate_vertical_motion(&self, ctx: &StateProcessingContext) -> f32 {
        let jump_phase = ctx.in_state_time as f32 / JUMP_DURATION as f32;
        let jump_curve = (std::f32::consts::PI * jump_phase).sin(); // Smooth jump curve

        // Scale jump height based on goalkeeper's jumping ability
        let max_height = JUMP_HEIGHT * (ctx.player.skills.physical.jumping as f32 / 20.0);

        jump_curve * max_height
    }
}