use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::forwarders::states::ForwardState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;
use crate::r#match::events::Event;

static FORWARD_CROSS_RECEIVING_STATE_NETWORK: LazyLock<NeuralNetwork> = LazyLock::new(|| {
    DefaultNeuralNetworkLoader::load(include_str!("nn_cross_receiving_data.json"))
});

#[derive(Default)]
pub struct ForwardCrossReceivingState {}

impl StateProcessingHandler for ForwardCrossReceivingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let ball_ops = ctx.ball();

        if !ball_ops.is_towards_player_with_angle(0.8) || ctx.ball().distance() > 100.0 {
            return Some(StateChangeResult::with_forward_state(ForwardState::Running));
        }

        if ball_ops.distance() <= self.receiving_range() {
            return Some(StateChangeResult::with_event(Event::PlayerEvent(PlayerEvent::RequestBallReceive(ctx.player.id))));
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

impl ForwardCrossReceivingState {
    fn calculate_target_position(&self, ctx: &StateProcessingContext) -> Vector3<f32> {
        let ball_position = ctx.tick_context.positions.ball.position;
        let goal_position = ctx.ball().direction_to_opponent_goal();

        // Calculate the target position within the crossing zone
        let crossing_zone_width = 30.0; // Adjust based on your game's scale
        let crossing_zone_length = 20.0; // Adjust based on your game's scale
        let target_x = ball_position.x + (rand::random::<f32>() - 0.5) * crossing_zone_width;
        let target_y = goal_position.y - crossing_zone_length / 2.0;

        Vector3::new(target_x, target_y, 0.0)
    }

    fn receiving_range(&self) -> f32 {
        2.0 // Adjust based on your game's scale
    }
}
