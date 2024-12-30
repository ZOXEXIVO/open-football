use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::Event;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;
use std::sync::LazyLock;

static GOALKEEPER_DISTRIBUTING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_distributing_data.json")));

#[derive(Default)]
pub struct GoalkeeperDistributingState {}

impl StateProcessingHandler for GoalkeeperDistributingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // 1. Check if the goalkeeper has the ball
        if !ctx.player.has_ball(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ));
        }

        if let Some(teammate_id) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_goalkeeper_state_and_event(
                GoalkeeperState::ReturningToGoal,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.tick_context.positions.players.position(teammate_id))
                        .with_force(ctx.player().pass_teammate_power(teammate_id))
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

impl GoalkeeperDistributingState {
    fn find_best_teammate_to_distribute(&self, ctx: &StateProcessingContext) -> Option<u32> {
        let players = ctx.players();

        if let Some((teammate_id, _)) = players
            .teammates()
            .nearby_ids(150.0)
            .choose(&mut rand::thread_rng())
        {
            return Some(teammate_id);
        }

        None
    }

    fn is_in_good_scoring_position(&self, ctx: &StateProcessingContext, player_id: u32) -> bool {
        // TODO
        let distance_to_goal = ctx.ball().distance_to_opponent_goal();
        distance_to_goal < 20.0 // Adjust based on your game's scale
    }

    fn find_best_pass_option<'a>(&'a self, ctx: &'a StateProcessingContext<'a>) -> Option<u32> {
        let players = ctx.players();

        if let Some((teammate_id, _)) = players
            .teammates()
            .nearby_ids(300.0)
            .choose(&mut rand::thread_rng())
        {
            return Some(teammate_id);
        }

        None
    }

    pub fn calculate_pass_power(&self, teammate_id: u32, ctx: &StateProcessingContext) -> f64 {
        let distance = ctx.tick_context.distances.get(ctx.player.id, teammate_id);

        let pass_skill = ctx.player.skills.technical.passing;

        (distance / pass_skill as f32 * 10.0) as f64
    }
}
