use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::events::Event;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _FORWARD_FINISHING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_finishing_data.json")));

#[derive(Default)]
pub struct ForwardFinishingState {}

impl StateProcessingHandler for ForwardFinishingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if !ctx.player.has_ball(ctx) {
            // Transition to Running state if the player doesn't have the ball
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        // Check if the player is within shooting range
        if !self.is_within_shooting_range(ctx) {
            // Transition to Dribbling state if the player is not within shooting range
            return Some(StateChangeResult::with_forward_state(
                ForwardState::Dribbling,
            ));
        }

        // Check if there's a clear shot on goal
        if !ctx.player().has_clear_shot() {
            // Transition to Passing state if there's no clear shot on goal
            return Some(StateChangeResult::with_forward_state(ForwardState::Passing));
        }

        // Calculate the shooting direction and power
        let (shooting_direction, _) = self.calculate_shooting_parameters(ctx);

        // Transition to Running state after taking the shot
        Some(StateChangeResult::with_forward_state_and_event(
            ForwardState::Running,
            Event::PlayerEvent(PlayerEvent::RequestShot(ctx.player.id, shooting_direction)),
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

impl ForwardFinishingState {
    fn is_within_shooting_range(&self, ctx: &StateProcessingContext) -> bool {
        ctx.ball().distance_to_opponent_goal() <= 150.0
    }

    fn calculate_shooting_parameters(&self, ctx: &StateProcessingContext) -> (Vector3<f32>, f32) {
        let goal_position = ctx.ball().direction_to_opponent_goal();
        let shooting_direction = (goal_position - ctx.player.position).normalize();
        let shooting_power = 1.0; // Adjust based on your game's mechanics

        (shooting_direction, shooting_power)
    }
}
