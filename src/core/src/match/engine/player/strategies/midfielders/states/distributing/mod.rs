use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, PlayerSide, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, VectorExtensions,
};
use nalgebra::Vector3;
use rand::prelude::IteratorRandom;
use rand::thread_rng;
use std::sync::LazyLock;

static _MIDFIELDER_DISTRIBUTING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_distributing_data.json")));

#[derive(Default)]
pub struct MidfielderDistributingState {}

impl StateProcessingHandler for MidfielderDistributingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Find the best passing option
        if let Some(teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Running,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_player_id(ctx.player.id)
                        .with_target(ctx.tick_context.positions.players.position(teammate.id))
                        .with_force(ctx.player().pass_teammate_power(teammate.id))
                        .build(),
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

impl MidfielderDistributingState {
    fn find_best_pass_option<'a>(
        &self,
        ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        let players = ctx.players();
        let teammates = players.teammates();

        let vision_range = ctx.player.skills.mental.vision * 10.0; // Adjust the factor as needed

        let open_teammates: Vec<MatchPlayerLite> = teammates
            .nearby(vision_range)
            .filter(|t| !t.tactical_positions.is_goalkeeper())
            .filter(|t| self.is_teammate_open(ctx, t))
            .collect();

        if !open_teammates.is_empty() {
            let best_option = open_teammates
                .iter()
                .max_by(|a, b| {
                    let space_a = self.calculate_space_around_player(ctx, a);
                    let space_b = self.calculate_space_around_player(ctx, b);
                    space_a.partial_cmp(&space_b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned();

            best_option
        } else {
            None
        }
    }

    fn is_teammate_open(&self, ctx: &StateProcessingContext, teammate: &MatchPlayerLite) -> bool {
        let opponent_distance_threshold = 5.0; // Adjust the threshold as needed

        let opponents_nearby = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                let distance = (opponent.position - teammate.position).magnitude();
                distance <= opponent_distance_threshold
            })
            .count();

        opponents_nearby == 0
    }

    fn calculate_space_around_player(
        &self,
        ctx: &StateProcessingContext,
        player: &MatchPlayerLite,
    ) -> f32 {
        let space_radius = 10.0; // Adjust the radius as needed

        let num_opponents_nearby = ctx
            .players()
            .opponents()
            .all()
            .filter(|opponent| {
                let distance = (opponent.position - player.position).magnitude();
                distance <= space_radius
            })
            .count();

        space_radius - num_opponents_nearby as f32
    }
}
