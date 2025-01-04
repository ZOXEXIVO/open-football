use serde::{Deserialize, Serialize};
use nalgebra::{DMatrix, DVector};

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

    pub fn save_json(&self) -> String {
        serde_json::to_string(&self).unwrap()
    }

    pub fn load_json(json: &str) -> NeuralNetwork {
        serde_json::from_str(json).unwrap()
    }

    pub fn run(&self, inputs: &DVector<f64>) -> DVector<f64> {
        self.layers
            .iter()
            .fold(inputs.clone(), |inputs, layer| layer.run(&inputs))
    }

    #[inline]
    pub fn activate(&self, x: f64, activation_func: ActivationFunction) -> f64 {
        match activation_func {
            ActivationFunction::Sigmoid => sigmoid(x),
            ActivationFunction::Relu => relu(x),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct LayerConfiguration {
    pub neurons_count: usize,
    pub activation_fn: ActivationFunction,
}

impl LayerConfiguration {
    pub fn new(neurons_count: usize, activation_fn: ActivationFunction) -> LayerConfiguration {
        LayerConfiguration {
            neurons_count,
            activation_fn,
        }
    }
}

impl From<usize> for LayerConfiguration {
    fn from(x: usize) -> Self {
        LayerConfiguration {
            neurons_count: x,
            activation_fn: ActivationFunction::Relu,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Layer {
    pub weights: DMatrix<f64>,
    pub bias: DVector<f64>,
    pub activation_fn: ActivationFunction,
}

impl Layer {
    pub fn new(configuration: LayerConfiguration, inputs: usize) -> Layer {
        Layer {
            weights: DMatrix::new_random(configuration.neurons_count, inputs),
            bias: DVector::new_random(configuration.neurons_count),
            activation_fn: configuration.activation_fn,
        }
    }

    pub fn run(&self, inputs: &DVector<f64>) -> DVector<f64> {
        (&self.weights * inputs + &self.bias).map(|x| self.activate(x))
    }

    #[inline]
    fn activate(&self, x: f64) -> f64 {
        match self.activation_fn {
            ActivationFunction::Sigmoid => sigmoid(x),
            ActivationFunction::Relu => relu(x),
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
    Relu
}

#[inline]
pub fn sigmoid(x: f64) -> f64 {
    1f64 / (1f64 + (-x).exp())
}

#[inline]
pub fn relu(x: f64) -> f64 {
    f64::max(0.0, x)
}