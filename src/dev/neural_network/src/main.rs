use std::fs;
use std::sync::Mutex;
use core::ActivationFunction;
use crate::train::Trainer;
use core::LayerConfiguration;
use core::NeuralNetwork;
use nalgebra::DVector;
use rayon::prelude::*;

mod train;

fn train(
    training_set: &TrainingSet,
    training_data: &[(DVector<f64>, DVector<f64>)]
) -> (NeuralNetwork, f64) {
    let mut net = NeuralNetwork::new(&training_set.configuration);

    let error_rate = net.train(training_data, training_set.learning_rate, training_set.momentum, training_set.epochs);

    (net, error_rate)
}

fn main() {
    let training_data =[
        (DVector::from(vec![0f64, 0f64, 0f64]), DVector::from(vec![0f64])),

        (DVector::from(vec![1f64, 0f64, 0f64]), DVector::from(vec![0f64])),
        (DVector::from(vec![0f64, 1f64, 0f64]), DVector::from(vec![0f64])),
        (DVector::from(vec![0f64, 0f64, 1f64]), DVector::from(vec![0f64])),

        (DVector::from(vec![1f64, 1f64, 0f64]), DVector::from(vec![0f64])),
        (DVector::from(vec![0f64, 1f64, 1f64]), DVector::from(vec![1f64])),
        (DVector::from(vec![1f64, 0f64, 1f64]), DVector::from(vec![0f64])),

        (DVector::from(vec![1f64, 1f64, 1f64]), DVector::from(vec![1f64])),
    ];

    let mut configurations = Vec::new();

    let max_length = 10u32;

    for momentum in &[0.1, 0.15f64, 0.2f64] {
        for rate in &[0.3, 0.2, 0.1, 0.05, 0.01] {
            for epochs in &[100000] {
                for first in 0..max_length {
                    for second in 0..max_length {
                        for third in 0..max_length {
                            for fourth in 0..max_length {
                                for fifth in 0..max_length {
                                    let mut layer_configuration = vec![
                                        LayerConfiguration::new(3, ActivationFunction::Relu)
                                    ];

                                    if first > 0 {
                                        layer_configuration.push(LayerConfiguration::new(first as usize, ActivationFunction::Relu));
                                    }

                                    if second > 0 {
                                        layer_configuration.push(LayerConfiguration::new(second as usize, ActivationFunction::Relu));
                                    }

                                    if third > 0 {
                                        layer_configuration.push(LayerConfiguration::new(third as usize, ActivationFunction::Relu));
                                    }

                                    if fourth > 0 {
                                        layer_configuration.push(LayerConfiguration::new(fourth as usize, ActivationFunction::Relu));
                                    }

                                    if fifth > 0 {
                                        layer_configuration.push(LayerConfiguration::new(fifth as usize, ActivationFunction::Relu));
                                    }
                                    
                                    layer_configuration.push(LayerConfiguration::new(1, ActivationFunction::Relu));

                                    configurations.push(TrainingSet {
                                        learning_rate: *rate,
                                        momentum: *momentum,
                                        epochs: *epochs,
                                        configuration: layer_configuration
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let ratings: Mutex<Vec<(TrainingSet, f64, NeuralNetwork)>> = Mutex::new(Vec::new());

    configurations.par_iter().for_each(|training_set| {
        let (nn, error) = train(training_set, &training_data);

        ratings.lock().unwrap().push((training_set.clone(), error, nn));
    });

    let mut ratings_lock = ratings.lock().unwrap();

    ratings_lock.sort_by(|(_, error, _), (_, next_error, _)| { error.partial_cmp(next_error).unwrap() });

    for (index, (training_set, error, _nn)) in ratings_lock.iter().take(10).enumerate() {
        let neurons: Vec<String> = training_set.configuration.iter().map(|t| t.neurons_count.to_string()).collect();
        let joined_str = format!("[{}]", neurons.join(","));

        println!("{}) {:?} - {} (epochs: {}, learning_rate: {}, momentum: {})", index + 1, joined_str, error, training_set.epochs, training_set.learning_rate, training_set.momentum);
    }

    let (_, error, best_nn) = ratings_lock.first().unwrap();

    println!("Results on best Neural Network (ERROR = {}):", error);

    for (training_item, training_result) in &training_data {
        let best_nn_res = best_nn.run(training_item);
        println!("DATA: {:?}, RESULT: {:?}, EXPECTED: {:?}", training_item.as_slice(), best_nn_res.as_slice(), training_result.as_slice());
    }

    let nn_json = best_nn.save_json();
    fs::write("best_nn.json", &nn_json).expect("Unable to write nn file");
}

#[derive(Clone, Debug)]
pub struct TrainingSet{
    learning_rate: f64,
    momentum: f64,
    epochs: u32,
    configuration: Vec<LayerConfiguration>
}