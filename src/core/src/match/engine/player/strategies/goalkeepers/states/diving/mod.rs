use std::sync::LazyLock;
use nalgebra::Vector3;
use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::{StateChangeResult, StateProcessingContext, StateProcessingHandler};

static GOALKEEPER_DIVING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_diving_data.json")));

#[derive(Default)]
pub struct GoalkeeperDivingState {}

impl StateProcessingHandler for GoalkeeperDivingState {
    fn try_fast(&self, context: &mut StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn process_slow(&self, context: &mut StateProcessingContext) -> StateChangeResult {
        StateChangeResult::none()
    }

    fn velocity(&self) -> Vector3<f32> {
        Vector3::new(0.0, 0.0, 0.0)
    }
}
