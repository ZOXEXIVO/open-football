use crate::common::loader::DefaultNeuralNetworkLoader;
use crate::common::NeuralNetwork;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::{
    ConditionContext, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;
use std::sync::LazyLock;

static _MIDFIELDER_TRACKING_RUNNER_STATE_NETWORK: LazyLock<NeuralNetwork> = LazyLock::new(|| {
    DefaultNeuralNetworkLoader::load(include_str!("nn_tracking_runner_data.json"))
});

const TRACKING_DISTANCE_THRESHOLD: f32 = 10.0; // Maximum distance to track the runner
const STAMINA_THRESHOLD: f32 = 50.0; // Minimum stamina required to continue tracking

#[derive(Default)]
pub struct MidfielderTrackingRunnerState {}

impl StateProcessingHandler for MidfielderTrackingRunnerState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let players = ctx.players();
        let opponents = players.opponents();

        let nearest_forward = opponents.forwards().min_by(|a, b| {
            let dist_a = (a.position - ctx.player.position).magnitude();
            let dist_b = (b.position - ctx.player.position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        });

        if let Some(runner) = nearest_forward {
            // Check if the midfielder has enough stamina to continue tracking
            if ctx.player.player_attributes.condition_percentage() < STAMINA_THRESHOLD as u32 {
                // If stamina is low, transition to the Defending state
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Check if the runner is within tracking distance
            let distance_to_runner = (ctx.player.position - runner.position).magnitude();
            if distance_to_runner > TRACKING_DISTANCE_THRESHOLD {
                // If the runner is too far, transition to the Defending state
                return Some(StateChangeResult::with_midfielder_state(
                    MidfielderState::Returning,
                ));
            }

            // Continue tracking the runner
            None
        } else {
            // If no opponent runner is found, transition to the Defending state
            Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Returning,
            ))
        }
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        let players = ctx.players();
        let opponents = players.opponents();
        let forwards = opponents.forwards();

        let nearest_forward = forwards.min_by(|a, b| {
            let dist_a = (a.position - ctx.player.position).magnitude();
            let dist_b = (b.position - ctx.player.position).magnitude();
            dist_a.partial_cmp(&dist_b).unwrap()
        });

        // Move towards the opponent runner
        if let Some(runner) = nearest_forward {
            let steering = SteeringBehavior::Pursuit {
                target: runner.position
            }
            .calculate(ctx.player);

            Some(steering.velocity)
        } else {
            // If no runner is found, stay in the current position
            Some(Vector3::new(0.0, 0.0, 0.0))
        }
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderTrackingRunnerState {}
