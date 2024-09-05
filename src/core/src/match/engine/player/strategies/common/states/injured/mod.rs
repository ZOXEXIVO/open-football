use std::sync::LazyLock;

use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::{StateProcessingContext, StateProcessingHandler};
use crate::r#match::strategies::processing::StateChangeResult;

static COMMON_INJURED_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_common_injured_data.json")));

#[derive(Default)]
pub struct CommonInjuredState {}

impl StateProcessingHandler for CommonInjuredState {
    fn try_fast(&self, context: &mut StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn process_slow(&self, context: &mut StateProcessingContext) -> StateChangeResult {
        StateChangeResult::none()
    }
}
