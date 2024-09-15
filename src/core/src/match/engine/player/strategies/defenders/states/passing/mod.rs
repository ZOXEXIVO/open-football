use std::sync::LazyLock;
use nalgebra::Vector3;
use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::{
    StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::player::events::PlayerUpdateEvent;

static DEFENDER_PASSING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_passing_data.json")));

#[derive(Default)]
pub struct DefenderPassingState {}

impl StateProcessingHandler for DefenderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball {
            return Some(StateChangeResult::with_defender_state(DefenderState::Standing));
        }

        let (nearest_teammates, opponents) = ctx.tick_context
            .objects_positions
            .player_distances
            .players_within_distance(ctx.player, 30.0);

        if opponents.len() > 1  {
            return Some(StateChangeResult::with_defender_state(DefenderState::Clearing));
        }

        let mut best_player_id = None;
        let mut highest_score = 0.0;

        for (player_id, teammate_distance) in nearest_teammates {
            let score = 1.0 / (teammate_distance + 1.0);
            if score > highest_score {
                highest_score = score;
                best_player_id = Some(player_id);
            }
        }

        if let Some(player_id) = best_player_id {
            if let Some(teammate_player_position) = ctx.tick_context.objects_positions.players_positions.get_player_position(player_id) {
                let mut events = ctx.result.borrow_mut();
                events.push(PlayerUpdateEvent::PassTo(teammate_player_position, 0.0));
            }
        }

        None
    }

    fn process_slow(&self, ctx: &StateProcessingContext) -> StateChangeResult {
        StateChangeResult::none()
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }
}
