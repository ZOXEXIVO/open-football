use core::ActivationFunction;
use core::NeuralNetwork;
use core::Layer;
use std::iter::{Enumerate, Zip};
use std::ops::{IndexMut};
use std::slice;

pub trait Trainer {
    fn train(&mut self, training_data: &[(Vec<f64>, Vec<f64>)], learning_rate: f64, momentum: f64, epochs: u32) -> f64;
    fn error(&self, run_results: &[Vec<f64>], required_values: &Vec<f64>) -> f64;
    fn weight_updates(&self, results: &[Vec<f64>], targets: &[f64]) -> Vec<Vec<Vec<f64>>>;
    fn updates_weights(&mut self, weight_updates: Vec<Vec<Vec<f64>>>, deltas: &mut Vec<Vec<Vec<f64>>>, learning_rate: f64, momentum: f64);
    fn initial_deltas(&self) -> Vec<Vec<Vec<f64>>>;
}

impl Trainer for NeuralNetwork {
    fn train(&mut self, training_data: &[(Vec<f64>, Vec<f64>)], learning_rate: f64, momentum: f64, epochs: u32) -> f64 {
        let mut deltas = self.initial_deltas();
        let mut error_rate = 0f64;

        for epoch in 0..epochs {
            error_rate = 0f64;

            for (input, target) in training_data.iter() {

                let run_results = self.run_internal(input);

                error_rate += self.error(&run_results, target);

                self.updates_weights(
                    self.weight_updates(&run_results, target),
                    &mut deltas,
                    learning_rate,
                    momentum,
                );
            }
        }
        
        error_rate
    }

    fn error(&self, run_results: &[Vec<f64>], required_values: &Vec<f64>) -> f64 {
        let mut error = 0f64;
        let output_layer_result = run_results.last().unwrap();
        let output_layer = self.layers.last().unwrap();

        for (&result, &target_output) in output_layer_result.iter().zip(required_values) {
            let activated_result = activate(result, output_layer.activation_fn);
            error += (target_output - activated_result).powi(2);
        }

        error
    }

    fn weight_updates(&self, results: &[Vec<f64>], targets: &[f64]) -> Vec<Vec<Vec<f64>>> {
        let mut network_errors: Vec<Vec<f64>> = Vec::new();
        let mut network_weight_updates = Vec::new();

        let layers = &self.layers;
        let network_results = &results[1..];

        let mut next_layer_nodes: Option<&Layer> = None;

        for (layer_index, (layer_nodes, layer_results)) in iter_zip_enum(layers, network_results).rev() {
            let prev_layer_results = &results[layer_index];
            let mut layer_errors = Vec::with_capacity(layer_nodes.neurons.len());
            let mut layer_weight_updates = Vec::with_capacity(layer_nodes.neurons.len());

            for (node_index, (neuron, &result)) in layer_nodes.neurons.iter().zip(layer_results).enumerate() {
                let mut node_weight_updates = Vec::with_capacity(neuron.weights.len());
                let node_error: f64;

                if layer_index == layers.len() - 1 {
                    let activated_result = activate(result, layer_nodes.activation_fn);
                    node_error = activated_result * (1f64 - activated_result) * (targets[node_index] - activated_result);
                } else {
                    let mut sum = 0f64;
                    let next_layer_errors = &network_errors[network_errors.len() - 1];
                    for (next_node, &next_node_error_data) in next_layer_nodes.unwrap().neurons.iter().zip((next_layer_errors).iter()) {
                        sum += next_node.weights[node_index + 1] * next_node_error_data;
                    }
                    let activated_result = activate(result, layer_nodes.activation_fn);
                    node_error = activated_result * (1f64 - activated_result) * sum;
                }

                for weight_index in 0..neuron.weights.len() {
                    let prev_layer_result = if weight_index == 0 {
                        1f64
                    } else {
                        prev_layer_results[weight_index - 1]
                    };
                    let weight_update = node_error * prev_layer_result;
                    node_weight_updates.push(weight_update);
                }

                layer_errors.push(node_error);
                layer_weight_updates.push(node_weight_updates);
            }

            network_errors.push(layer_errors);
            network_weight_updates.push(layer_weight_updates);
            next_layer_nodes = Some(&layer_nodes);
        }

        network_weight_updates.reverse();
        network_weight_updates
    }

    fn updates_weights(
        &mut self,
        weight_updates: Vec<Vec<Vec<f64>>>,
        deltas: &mut Vec<Vec<Vec<f64>>>,
        learning_rate: f64,
        momentum: f64,
    ) {
        for layer_index in 0..self.layers.len() {
            let layer = &mut self.layers[layer_index];
            let layer_weight_updates = &weight_updates[layer_index];

            for neuron_index in 0..layer.neurons.len() {
                let neuron = &mut layer.index_mut(neuron_index as u32);
                let neuron_weight_updates = &layer_weight_updates[neuron_index];

                for weight_index in 0..neuron.weights.len() {
                    let weight_update = neuron_weight_updates[weight_index];
                    let prev_delta = deltas[layer_index][neuron_index][weight_index];
                    let delta = (learning_rate * weight_update) + (momentum * prev_delta);
                    neuron.weights[weight_index] += delta;
                    deltas[layer_index][neuron_index][weight_index] = delta;
                }
            }
        }
    }

    fn initial_deltas(&self) -> Vec<Vec<Vec<f64>>> {
        self.layers
            .iter()
            .map(|layer| {
                vec![vec![0f64; layer.neurons[0].weights.len()]; layer.neurons.len()]
            })
            .collect()
    }
}

fn iter_zip_enum<'s, 't, S: 's, T: 't>(
    s: &'s [S],
    t: &'t [T],
) -> Enumerate<Zip<slice::Iter<'s, S>, slice::Iter<'t, T>>> {
    s.iter().zip(t.iter()).enumerate()
}

fn activate(x: f64, activation_func: ActivationFunction) -> f64 {
    match activation_func {
        ActivationFunction::Sigmoid => sigmoid(x),
        ActivationFunction::Relu => relu(x),
        ActivationFunction::Tanh => tanh(x),
        ActivationFunction::Softmax => panic!("Softmax should be applied to the entire layer, not individual neurons"),
    }
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
pub fn tanh(x: f64) -> f64 {
    x.tanh()
}