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
                MidfielderState::Returning,
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

        let nearby_teammates = teammates.nearby(300.0);

        let mut nearest_to_goal: Option<MatchPlayerLite> = None;
        let mut min_to_goal_distance = f32::MAX;

        for teammate in nearby_teammates {
            let player = ctx
                .context
                .players
                .by_id(teammate.id)
                .expect(&format!("can't find player with id = {}", teammate.id));

            if player.tactical_position.current_position.is_goalkeeper() {
                continue;
            }

            let opponent_goal_position = match ctx.player.side {
                Some(PlayerSide::Left) => ctx.context.goal_positions.right,
                Some(PlayerSide::Right) => ctx.context.goal_positions.left,
                _ => Vector3::new(0.0, 0.0, 0.0),
            }
            .distance_to(&player.position);

            if opponent_goal_position < min_to_goal_distance {
                min_to_goal_distance = opponent_goal_position;
                nearest_to_goal = Some(teammate);
            }
        }

        nearest_to_goal
    }
}
