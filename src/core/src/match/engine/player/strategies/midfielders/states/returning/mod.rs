use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, PlayerDistanceFromStartPosition, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

const STAMINA_THRESHOLD: u32 = 20; // Minimum stamina percentage before resting

static MIDFIELDER_RETURNING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_returning_data.json")));

#[derive(Default)]
pub struct MidfielderReturningState {}

impl StateProcessingHandler for MidfielderReturningState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.ball().distance() < 250.0 && ctx.ball().is_towards_player_with_angle(0.9) {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Intercepting,
            ));
        }
        
        if ctx.ball().distance() < 10.0 {
            // Transition to Tackling state to attempt to regain possession
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Tackling,
            ));
        }

        // 1. Check if the midfielder has reached their starting position
        if ctx.player().position_to_distance() == PlayerDistanceFromStartPosition::Small {
            // Transition to Standing state
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Walking,
            ));
        }

        // 3. Check if the team is losing and time is running out
        if ctx.team().is_loosing() && ctx.context.time.is_running_out() {
            // Transition to SupportingAttack to push forward
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::AttackSupporting,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Arrive {
                target: ctx.player.start_position + ctx.player().separation_velocity(),
                slowing_distance: 10.0,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}
