use serde::{Deserialize, Serialize};
use std::ops::{Index, IndexMut};

#[derive(Debug, Serialize, Deserialize)]
pub struct NeuralNetwork {
    pub layers: Vec<Layer>,
}

impl NeuralNetwork {
    pub fn new(layers_configurations: &[LayerConfiguration]) -> Self {
        let layers_len = layers_configurations.len();

        let mut layers = Vec::with_capacity(layers_len);

        for idx in 0..layers_len {
            let current_layer_config = layers_configurations[idx];
            let current_inputs = layers_configurations[{
                // case for first layer (no previous)
                match idx {
                    0 => idx,
                    _ => idx - 1,
                }
            }];

            layers.push(Layer::new(
                current_layer_config,
                current_inputs.neurons_count,
            ));
        }

        NeuralNetwork { layers }
    }

    pub fn load_json(json: &str) -> NeuralNetwork {
        serde_json::from_str(json).unwrap()
    }

    pub fn run(&self, inputs: &[f64]) -> Vec<f64> {
        // run network
        let mut results = self.run_internal(inputs);

        // latest output will be at the end of vec
        results.pop().unwrap()
    }

    pub fn run_internal(&self, inputs: &[f64]) -> Vec<Vec<f64>> {
        let mut results = Vec::with_capacity(self.layers.len());

        // Fill first layer
        results.push(inputs.to_vec());

        for (layer_idx, layer) in self.layers.iter().enumerate() {
            let mut layer_results = Vec::with_capacity(layer.neurons.len());

            // calculate weight * input
            for neuron in &layer.neurons {
                let mut total: f64 = neuron.weights[0];

                let current_result = &results[layer_idx];

                for (&weight, &value) in neuron.weights.iter().skip(1).zip(current_result) {
                    total += weight * value;
                }

                layer_results.push(total);
            }

            match layer.activation_fn {
                ActivationFunction::Sigmoid => {
                    layer_results.iter_mut().for_each(|x| *x = sigmoid(*x));
                }
                ActivationFunction::Relu => {
                    layer_results.iter_mut().for_each(|x| *x = relu(*x));
                }
                ActivationFunction::Softmax => {
                    let softmax_results = softmax(&layer_results);
                    layer_results = softmax_results;
                }
                ActivationFunction::Tanh => {
                    layer_results.iter_mut().for_each(|x| *x = tanh(*x));
                }
            }

            results.push(layer_results);
        }

        results
    }

    #[inline]
    fn activate(&self, x: f64, activation_func: ActivationFunction) -> f64 {
        match activation_func {
            ActivationFunction::Sigmoid => sigmoid(x),
            ActivationFunction::Relu => relu(x),
            ActivationFunction::Softmax => x,
            ActivationFunction::Tanh => tanh(x)
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LayerConfiguration {
    pub neurons_count: u32,
    pub activation_fn: ActivationFunction,
}

impl LayerConfiguration {
    pub fn new(neurons_count: u32, activation_fn: ActivationFunction) -> LayerConfiguration {
        LayerConfiguration {
            neurons_count,
            activation_fn,
        }
    }
}

impl From<u32> for LayerConfiguration {
    fn from(x: u32) -> Self {
        LayerConfiguration {
            neurons_count: x,
            activation_fn: ActivationFunction::Relu,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Layer {
    pub neurons: Vec<Neuron>,
    pub activation_fn: ActivationFunction,
}

impl Layer {
    pub fn new(configuration: LayerConfiguration, inputs: u32) -> Layer {
        Layer {
            neurons: (0..configuration.neurons_count)
                .map(|_| Neuron::new(inputs))
                .collect(),
            activation_fn: configuration.activation_fn,
        }
    }
}

impl Index<u32> for Layer {
    type Output = Neuron;

    fn index(&self, index: u32) -> &Self::Output {
        &self.neurons[index as usize]
    }
}

impl IndexMut<u32> for Layer {
    fn index_mut(&mut self, index: u32) -> &mut Self::Output {
        &mut self.neurons[index as usize]
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Neuron {
    pub weights: Vec<f64>,
}

impl Neuron {
    pub fn new(inputs: u32) -> Self {
        Neuron {
            weights: (0..=inputs).map(|_| random_f64()).collect(),
        }
    }
}

fn random_f64() -> f64 {
    2f64 * rand::random::<f64>() - 1f64
}

// Activations
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ActivationFunction {
    Sigmoid,
    Relu,
    Softmax,
    Tanh
}

#[inline]
pub fn sigmoid(x: f64) -> f64 {
    1f64 / (1f64 + (-x).exp())
}

#[inline]
pub fn relu(x: f64) -> f64 {
    f64::max(0.0, x)
}

#[inline]
pub fn softmax(values: &[f64]) -> Vec<f64> {
    let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = values.iter().map(|&v| (v - max_val).exp()).collect();
    let sum_exps: f64 = exps.iter().sum();

    exps.iter().map(|&v| v / sum_exps).collect()
}

#[inline]
pub fn tanh(x: f64) -> f64 {
    x.tanh()
}
