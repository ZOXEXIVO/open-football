use std::sync::LazyLock;

use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::{StateChangeResult, StateProcessingContext, StateProcessingHandler};

static GOALKEEPER_PUNCHING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_punching_data.json")));

#[derive(Default)]
pub struct GoalkeeperPunchingState {}

impl StateProcessingHandler for GoalkeeperPunchingState {
    fn try_fast(
        &self, context: &mut StateProcessingContext
    ) -> Option<StateChangeResult> {
        None
    }

    fn process_slow(
        &self, context: &mut StateProcessingContext
    ) -> StateChangeResult {
        StateChangeResult::none()
    }
}