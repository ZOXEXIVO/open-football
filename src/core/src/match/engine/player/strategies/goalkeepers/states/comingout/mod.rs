use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::goalkeepers::states::state::GoalkeeperState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext, StateProcessingHandler,
    SteeringBehavior,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _GOALKEEPER_COMINGOUT_STATE_NETWORK: LazyLock<NeuralNetwork> =
    LazyLock::new(|| DefaultNeuralNetworkLoader::load(include_str!("nn_comingout_data.json")));

const COMING_OUT_DISTANCE_THRESHOLD: f32 = 20.0; // Maximum distance from the goal to consider coming out

#[derive(Default)]
pub struct GoalkeeperComingOutState {}

impl StateProcessingHandler for GoalkeeperComingOutState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        if ctx.ball().distance() < 100.0 {
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::PreparingForSave,
            ));
        }

        // 1. Check if the ball is within the coming out distance threshold
        let ball_distance = ctx.ball().distance_to_own_goal();
        if ball_distance > COMING_OUT_DISTANCE_THRESHOLD {
            // Ball is too far, transition to appropriate state (e.g., ReturningToGoal)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        // 2. Check if there are any opponents near the ball
        let players = ctx.players();
        let opponents = players.opponents();
        if let Some(_) = opponents.with_ball().next() {
            // No opponents near the ball, transition to appropriate state (e.g., ReturningToGoal)
            return Some(StateChangeResult::with_goalkeeper_state(
                GoalkeeperState::ReturningToGoal,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        // Implement neural network processing if needed
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        Some(
            SteeringBehavior::Pursuit {
                target: ctx.tick_context.positions.ball.position,
            }
            .calculate(ctx.player)
            .velocity,
        )
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}
