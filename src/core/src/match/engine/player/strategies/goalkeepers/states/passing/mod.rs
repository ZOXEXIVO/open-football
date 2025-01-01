use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext, StateProcessingHandler};
use nalgebra::Vector3;
use std::sync::LazyLock;

static GOALKEEPER_PASSING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_passing_data.json")));

#[derive(Default)]
pub struct GoalkeeperPassingState {}

impl StateProcessingHandler for GoalkeeperPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_player_id(ctx.player.id)
                        .with_target(teammate.position)
                        .with_force(ctx.player().pass_teammate_power(teammate.id))
                        .build()
                )),
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperPassingState {
    fn find_best_pass_option(&self, ctx: &StateProcessingContext) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        teammates
            .nearby(200.0)
            .max_by(|a, b| {
                let opponents_near_a = players.opponents().nearby_raw(20.0).count();
                let opponents_near_b = players.opponents().nearby_raw(20.0).count();
                opponents_near_a.cmp(&opponents_near_b).reverse()
            })
    }
}
