use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _DEFENDER_BLOCKING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_blocking_data.json")));

const BLOCK_DISTANCE_THRESHOLD: f32 = 2.0; // Maximum distance to attempt a block (in meters)

#[derive(Default)]
pub struct DefenderBlockingState {}

impl StateProcessingHandler for DefenderBlockingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing
            ));
        }

        if ctx.ball().distance() > BLOCK_DISTANCE_THRESHOLD {
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

        if ctx.ball().is_towards_player_with_angle(0.9) {
            // Defender is not in the path of the ball
            return Some(StateChangeResult::with_defender_state(
                DefenderState::Standing,
            ));
        }

       None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network logic if necessary
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // Defender may need to adjust position slightly to attempt block
        // Calculate minimal movement towards the blocking position
        // For simplicity, we'll assume the defender remains stationary
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {
        // No additional conditions
    }
}
