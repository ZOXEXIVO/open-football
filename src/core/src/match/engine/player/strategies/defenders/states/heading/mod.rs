use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::player::events::{PlayerEvent, ShootingEventBuilder, ShootingEventContext};
use crate::r#match::{
    ConditionContext, PlayerSide, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;
use crate::r#match::events::Event;

static DEFENDER_HEADING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_heading_data.json")));

const HEADING_HEIGHT_THRESHOLD: f32 = 1.5; // Minimum height to consider heading (meters)
const HEADING_DISTANCE_THRESHOLD: f32 = 1.5; // Maximum distance to the ball for heading (meters)
const HEADING_SUCCESS_THRESHOLD: f32 = 0.5; // Threshold for heading success based on skills

#[derive(Default)]
pub struct DefenderHeadingState {}

impl StateProcessingHandler for DefenderHeadingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_position = ctx.tick_context.positions.ball.position;

        if ctx.ball().distance() > HEADING_DISTANCE_THRESHOLD {
            // Transition back to appropriate state (e.g., HoldingLine)
            return Some(StateChangeResult::with_defender_state(
                DefenderState::HoldingLine,
            ));
        }

        // Check if the ball is at a height suitable for heading
        if ball_position.z < HEADING_HEIGHT_THRESHOLD {
            // Ball is too low to head
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }
       
        // 2. Attempt to head the ball
        if self.attempt_heading(ctx) {
            Some(StateChangeResult::with_defender_state_and_event(DefenderState::HoldingLine, Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::build()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().opponent_goal_position())
                    .with_force(ctx.player().shoot_goal_power())
                    .build()
            ))))
        } else {
            // Heading failed; transition to appropriate state (e.g., Standing)
            Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Defender is stationary while attempting to head the ball
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process
    }
}

impl DefenderHeadingState {
    /// Determines if the defender successfully heads the ball based on skills and random chance.
    fn attempt_heading(&self, ctx: &StateProcessingContext) -> bool {
        let heading_skill = ctx.player.skills.technical.heading as f32 / 100.0; // Normalize skill to [0,1]
        let jumping_skill = ctx.player.skills.physical.jumping as f32 / 100.0;
        let overall_skill = (heading_skill + jumping_skill) / 2.0;

        // Simulate chance of success
        let random_value: f32 = rand::random(); // Generates a random float between 0.0 and 1.0

        overall_skill > (random_value + HEADING_SUCCESS_THRESHOLD)
    }
}
