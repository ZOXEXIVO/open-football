use core::relu;
use core::sigmoid;
use core::ActivationFunction;
use core::Layer;
use core::NeuralNetwork;
use nalgebra::{DMatrix, DVector, RowDVector};
use std::iter::{Enumerate, Zip};
use std::slice;

pub trait Trainer {
    fn train(
        &mut self,
        training_data: &[(DVector<f64>, DVector<f64>)],
        learning_rate: f64,
        momentum: f64,
        epochs: u32,
    ) -> f64;
    fn error(&self, run_result: &DVector<f64>, required_values: &DVector<f64>) -> f64;
    fn weight_updates(&self, results: &[DVector<f64>], targets: &DVector<f64>)
        -> Vec<DMatrix<f64>>;
    fn update_weights(
        &mut self,
        weight_updates: &[DMatrix<f64>],
        deltas: &mut [DMatrix<f64>],
        learning_rate: f64,
        momentum: f64,
    );
    fn initial_deltas(&self) -> Vec<DMatrix<f64>>;
}

impl Trainer for NeuralNetwork {
    fn train(
        &mut self,
        training_data: &[(DVector<f64>, DVector<f64>)],
        learning_rate: f64,
        momentum: f64,
        epochs: u32,
    ) -> f64 {
        let mut deltas = self.initial_deltas();
        let mut error_rate = 0f64;

        for _ in 0..epochs {
            error_rate = 0f64;

            for (input, target) in training_data.iter() {
                let run_result = self.run(input);
                error_rate += self.error(&run_result, target);
                let weight_updates = self.weight_updates(&[input.clone(), run_result], target);
                self.update_weights(&weight_updates, &mut deltas, learning_rate, momentum);
            }
        }

        error_rate
    }

    fn error(&self, run_result: &DVector<f64>, required_values: &DVector<f64>) -> f64 {
        let output_layer = self.layers.last().unwrap();
        let activated_result = run_result.map(|x| self.activate(x, output_layer.activation_fn));
        (activated_result - required_values)
            .map(|x| x.powi(2))
            .sum()
    }

    fn weight_updates(
        &self,
        results: &[DVector<f64>],
        targets: &DVector<f64>,
    ) -> Vec<DMatrix<f64>> {
        let mut weight_updates = Vec::new();
        let mut deltas = Vec::new();

        // Calculate error for output layer first
        let output_layer_result = results.last().unwrap();
        let output_error = output_layer_result.component_mul(
            &(DVector::from_element(output_layer_result.len(), 1.0) - output_layer_result)
        ).component_mul(
            &(targets - output_layer_result)
        );
        deltas.push(output_error);

        // Backpropagate through hidden layers
        for layer_idx in (0..self.layers.len() - 1).rev() {
            let next_layer = &self.layers[layer_idx + 1];
            let current_layer = &self.layers[layer_idx];
            let layer_output = &results[1];

            let prev_delta = deltas.last().unwrap();

            // Get weights without bias
            let weights_without_bias = next_layer.weights.columns(1, next_layer.weights.ncols() - 1);

            // Create delta vector with dimensions matching current layer
            let mut delta = DVector::zeros(current_layer.weights.nrows());

            // Calculate weighted error for each neuron in the current layer
            for i in 0..current_layer.weights.nrows() {
                let mut weighted_sum = 0.0;
                for j in 0..weights_without_bias.nrows() {
                    if j < prev_delta.len() && i < weights_without_bias.ncols() {
                        weighted_sum += weights_without_bias[(j, i)] * prev_delta[j];
                    }
                }

                // Calculate sigmoid derivative for this neuron
                let output_value = if i < layer_output.len() { layer_output[i] } else { 0.0 };
                let sigmoid_derivative = output_value * (1.0 - output_value);

                delta[i] = weighted_sum * sigmoid_derivative;
            }

            deltas.push(delta);
        }

        deltas.reverse();

        // Calculate weight updates for each layer
        for layer_idx in 0..self.layers.len() {
            let layer = &self.layers[layer_idx];
            let delta = &deltas[layer_idx];

            let layer_input = if layer_idx == 0 {
                &results[0]
            } else {
                &results[1]
            };

            // Create input vector with bias, ensuring correct dimensions
            let mut input_with_bias = DVector::zeros(layer.weights.ncols());  // Match weight matrix columns
            input_with_bias[0] = 1.0;  // Bias term

            // Copy input values after bias, ensuring we don't exceed vector length
            let copy_len = (layer.weights.ncols() - 1).min(layer_input.len());
            input_with_bias.rows_mut(1, copy_len).copy_from(&layer_input.rows(0, copy_len));

            // Calculate weight updates
            let update = delta * input_with_bias.transpose();
            
            weight_updates.push(update);
        }

        weight_updates
    }

    fn update_weights(
        &mut self,
        weight_updates: &[DMatrix<f64>],
        deltas: &mut [DMatrix<f64>],
        learning_rate: f64,
        momentum: f64,
    ) {
        for ((layer, layer_weight_updates), layer_deltas) in
            self.layers.iter_mut().zip(weight_updates).zip(deltas)
        {
            let delta =
                learning_rate * layer_weight_updates + momentum * layer_deltas.clone_owned();
            layer.weights += &delta;
            *layer_deltas = delta.clone();
        }
    }

    fn initial_deltas(&self) -> Vec<DMatrix<f64>> {
        self.layers
            .iter()
            .map(|layer| DMatrix::zeros(layer.weights.nrows(), layer.weights.ncols()))
            .collect()
    }
}

fn iter_zip_enum<'s, 't, S: 's, T: 't>(
    s: &'s [S],
    t: &'t [T],
) -> Enumerate<Zip<slice::Iter<'s, S>, slice::Iter<'t, T>>> {
    s.iter().zip(t.iter()).enumerate()
}
