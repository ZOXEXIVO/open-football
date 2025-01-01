use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _GOALKEEPER_CATCHING_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_catching_data.json")));

#[derive(Default)]
pub struct GoalkeeperCatchingState {}

impl StateProcessingHandler for GoalkeeperCatchingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if self.is_catch_successful(ctx) {
            let mut holding_result =
                StateChangeResult::with_goalkeeper_state(GoalkeeperState::HoldingBall);

            holding_result
                .events
                .add_player_event(PlayerEvent::CaughtBall(ctx.player.id));

            Some(holding_result)
        } else {
            Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::Standing,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        // During catching, the goalkeeper's velocity should be minimal
        // but we can add a small adjustment towards the ball
        let ball_position = ctx.tick_context.positions.ball.position;
        let direction = (ball_position - ctx.player.position).normalize();
        let speed = 0.5; // Very low speed for final adjustments

        Some(direction * speed)
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl GoalkeeperCatchingState {
    fn is_catch_successful(&self, ctx: &StateProcessingContext) -> bool {
        let catch_skill =
            (ctx.player.skills.technical.first_touch + ctx.player.skills.technical.technique) / 2.0;
        let ball_speed = ctx.tick_context.positions.ball.velocity.norm();
        let distance_to_ball = ctx.ball().distance();

        // Scale catch_skill from 1-20 range to 0-1 range
        let scaled_catch_skill = (catch_skill - 1.0) / 19.0;

        // Calculate catch probability based on scaled skill, ball speed, and distance
        let catch_probability =
            scaled_catch_skill * (1.0 - (ball_speed / 30.0)) * (1.0 - (distance_to_ball / 5.0));

        // Ensure catch probability is within the range of 0 to 1
        let clamped_catch_probability = catch_probability.clamp(0.0, 1.0);

        // Random number between 0 and 1
        let random_factor = rand::random::<f32>();

        clamped_catch_probability > random_factor
    }
}
