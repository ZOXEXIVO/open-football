use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::player::events::PlayerUpdateEvent;
use crate::r#match::strategies::processing::StateChangeResult;
use crate::r#match::{GameTickContext, MatchContext, MatchPlayer, PlayerTickContext};
use std::sync::LazyLock;

static GOALKEEPER_WALKING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_walking_data.json")));

pub struct GoalkeeperWalkingState {}

impl GoalkeeperWalkingState {
    pub fn process(
        in_state_time: u64,
        player: &mut MatchPlayer,
        context: &mut MatchContext,
        tick_context: &GameTickContext,
        player_context: PlayerTickContext,
        result: &mut Vec<PlayerUpdateEvent>,
    ) -> StateChangeResult {
        StateChangeResult::none()
    }
}