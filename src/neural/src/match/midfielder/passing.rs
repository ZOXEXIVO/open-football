use burn::config::Config;
use burn::nn::Initializer;
use burn::nn::{Linear, LinearConfig, Relu};
use std::sync::LazyLock;

use crate::{DefaultNeuralBackend, DEFAULT_NEURAL_DEVICE};
use burn::prelude::{Backend, Module};
use burn::record::{BinBytesRecorder, FullPrecisionSettings, Recorder};
use burn::tensor::Tensor;

static MODEL_BYTES: &[u8] = include_bytes!("model.bin");

pub static MIDFIELDER_PASSING_NEURAL_NETWORK: LazyLock<
    MidfielderPassingNeural<DefaultNeuralBackend>,
> = LazyLock::new(|| {
    let record = BinBytesRecorder::<FullPrecisionSettings>::default()
        .load(MODEL_BYTES.to_vec(), &DEFAULT_NEURAL_DEVICE)
        .expect("nn model load failed");

    MidfielderPassingNeuralConfig::init(&DEFAULT_NEURAL_DEVICE).load_record(record)
});

#[derive(Module, Debug)]
pub struct MidfielderPassingNeural<B: Backend> {
    linear_a: Linear<B>,
    linear_b: Linear<B>,
    linear_c: Linear<B>,
    linear_d: Linear<B>,

    activation: Relu,
}

unsafe impl<B> Sync for MidfielderPassingNeural<B> where B: Backend {}

impl<B: Backend> MidfielderPassingNeural<B> {
    pub fn forward(&self, input: Tensor<B, 2>) -> Tensor<B, 2> {
        let out = self.activation.forward(self.linear_a.forward(input));
        let out = self.activation.forward(self.linear_b.forward(out));
        let out = self.activation.forward(self.linear_c.forward(out));
        let out = self.activation.forward(self.linear_d.forward(out));

        out
    }
}

#[derive(Debug, Config)]
pub struct MidfielderPassingNeuralConfig {
    linear_a: LinearConfig,
    linear_b: LinearConfig,
    linear_c: LinearConfig,
    linear_d: LinearConfig,
}
 
impl MidfielderPassingNeuralConfig {
    pub fn init<B: Backend>(device: &B::Device) -> MidfielderPassingNeural<B> {
        MidfielderPassingNeural {
            linear_a: LinearConfig::new(2, 32)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            linear_b: LinearConfig::new(32, 32)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            linear_c: LinearConfig::new(32,16)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            linear_d: LinearConfig::new(16, 1)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            activation: Relu::new().to_owned(),
        }
    }
}