use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _GOALKEEPER_PICKING_UP_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_picking_up_data.json")));

const PICKUP_DISTANCE_THRESHOLD: f32 = 1.0; // Maximum distance to pick up the ball
const PICKUP_SUCCESS_PROBABILITY: f32 = 0.9; // Probability of successfully picking up the ball

#[derive(Default)]
pub struct GoalkeeperPickingUpState {}

impl StateProcessingHandler for GoalkeeperPickingUpState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the ball is within pickup distance
        let ball_distance = ctx.ball().distance();
        if ball_distance > PICKUP_DISTANCE_THRESHOLD {
            // Ball is too far to pick up, transition to appropriate state (e.g., Standing)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        // 2. Attempt to pick up the ball
        let pickup_success = rand::random::<f32>() < PICKUP_SUCCESS_PROBABILITY;
        if pickup_success {
            // Pickup is successful
            let mut state_change =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            // Generate a pickup event
            state_change
                .events
                .add_player_event(PlayerEvent::CaughtBall(ctx.player.id));

            return Some(state_change);
        } else {
            // Pickup failed, transition to appropriate state (e.g., Diving)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Diving,
            ));
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Move towards the ball to pick it up
        let ball_position = ctx.tick_context.positions.ball.position;
        let direction = (ball_position - ctx.player.position).normalize();
        let speed = ctx.player.skills.physical.pace;
        Some(direction * speed)
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions to process in this state
    }
}
