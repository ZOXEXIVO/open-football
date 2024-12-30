use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::defenders::states::DefenderState;
use crate::r#match::events::Event;
use crate::r#match::player::events::{PlayerEvent, ShootingEventContext};
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static FORWARD_SHOOTING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_shooting_data.json")));

#[derive(Default)]
pub struct DefenderShootingState {}

impl StateProcessingHandler for DefenderShootingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        Some(StateChangeResult::with_defender_state_and_event(
            DefenderState::Standing,
            Event::PlayerEvent(PlayerEvent::Shoot(
                ShootingEventContext::build()
                    .with_player_id(ctx.player.id)
                    .with_target(ctx.player().opponent_goal_position())
                    .with_force(ctx.player().shoot_goal_power())
                    .build()
            )),
        ))
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, _ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(Vector3::new(0.0, 0.0, 0.0))
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl DefenderShootingState {}
