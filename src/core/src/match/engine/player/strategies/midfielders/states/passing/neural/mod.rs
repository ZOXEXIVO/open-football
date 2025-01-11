use burn::nn::Initializer;
use burn::nn::{Linear, LinearConfig, Relu};
use burn::prelude::*;

use burn::prelude::Module;

#[derive(Module, Debug)]
pub struct MidfielderPassingNeural<B: Backend> {
    linear_a: Linear<B>,
    linear_b: Linear<B>,
    linear_c: Linear<B>,
    linear_d: Linear<B>,

    activation: Relu,
}

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
pub struct MyBinaryNetConfig {
    linear_a: LinearConfig,
    linear_b: LinearConfig,
    linear_c: LinearConfig,
    linear_d: LinearConfig,
}

impl MyBinaryNetConfig {
    pub fn init<B: Backend>(device: &B::Device) -> MidfielderPassingNeural<B> {
        MidfielderPassingNeural {
            linear_a: LinearConfig::new(2, 4)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            linear_b: LinearConfig::new(4, 8)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            linear_c: LinearConfig::new(8, 8)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            linear_d: LinearConfig::new(8, 1)
                .with_initializer(Initializer::Uniform { min: 0.0, max: 1.0 })
                .with_bias(true)
                .init(device),
            activation: Relu::new().to_owned()
        }
    }
}