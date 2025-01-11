pub mod neural;

pub use neural::*;

use crate::r#match::events::Event;
use crate::r#match::midfielders::states::MidfielderState;
use crate::r#match::player::events::{PassingEventContext, PlayerEvent};
use crate::r#match::{
    ConditionContext, MatchPlayerLite, StateChangeResult, StateProcessingContext,
    StateProcessingHandler, SteeringBehavior,
};
use nalgebra::Vector3;

use burn::backend::ndarray::NdArrayDevice;
use burn::backend::NdArray;
use burn::module::Module;
use burn::record::{BinBytesRecorder, FullPrecisionSettings, Recorder};
use std::sync::{Arc, OnceLock};
use burn::prelude::Tensor;

static MODEL_BYTES: &[u8] = include_bytes!("neural/model.bin");
static MIDFIELDER_PASSING_NEURAL_NETWORK: Arc<OnceLock<MidfielderPassingNeural<NdArray>>> = Arc::new();

#[derive(Default)]
pub struct MidfielderPassingState {}

impl StateProcessingHandler for MidfielderPassingState {
    fn try_fast(&self, ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        let next = MIDFIELDER_PASSING_NEURAL_NETWORK.get_or_init(|| {
            let device = NdArrayDevice::default();

            let record = BinBytesRecorder::<FullPrecisionSettings>::default()
                .load(MODEL_BYTES.to_vec(), &device)
                .expect("Should be able to load model the model weights from bytes");

            let model: MidfielderPassingNeural<NdArray> = MidfielderPassingNeuralConfig::init(&device);

            return Arc::new(model.load_record(record));
        });

        // Check if the midfielder still has the ball
        if !ctx.player.has_ball(ctx) {
            // Lost possession, transition to Pressing
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Pressing,
            ));
        }

        let device = NdArrayDevice::default();

        let tensor = Tensor::from_data([[1, 1]], &device);
        let result = next.forward(tensor);

        let tensor_data_string = result
            .to_data()
            .iter()
            .map(|x: f32| format!("{:.4}", x))
            .collect::<Vec<String>>()
            .join(", ");

        println!("### {}", tensor_data_string);

        // Determine the best teammate to pass to
        if let Some(target_teammate) = self.find_best_pass_option(ctx) {
            return Some(StateChangeResult::with_midfielder_state_and_event(
                MidfielderState::Standing,
                Event::PlayerEvent(PlayerEvent::PassTo(
                    PassingEventContext::build()
                        .with_from_player_id(ctx.player.id)
                        .with_to_player_id(target_teammate.id)
                        .with_target(target_teammate.position)
                        .with_force(ctx.player().pass_teammate_power(target_teammate.id))
                        .build(),
                )),
            ));
        }

        if ctx.ball().distance_to_opponent_goal() < 200.0 {
            return Some(StateChangeResult::with_midfielder_state(
                MidfielderState::Shooting,
            ));
        }

        None
    }

    fn process_slow(&self, _ctx: &StateProcessingContext) -> Option<StateChangeResult> {
        None
    }

    fn velocity(&self, ctx: &StateProcessingContext) -> Option<Vector3<f32>> {
        if let Some(nearest_teammate) = ctx.players().teammates().nearby_to_opponent_goal() {
            return Some(
                SteeringBehavior::Arrive {
                    target: nearest_teammate.position,
                    slowing_distance: 30.0,
                }
                .calculate(ctx.player)
                .velocity,
            );
        }

        None
    }

    fn process_conditions(&self, _ctx: ConditionContext) {}
}

impl MidfielderPassingState {
    fn find_best_pass_option<'a>(
        &self,
        _ctx: &StateProcessingContext<'a>,
    ) -> Option<MatchPlayerLite> {
        // TODO

        None
    }
}
