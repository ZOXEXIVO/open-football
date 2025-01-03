use core::ActivationFunction;
use crate::train::Trainer;
use core::LayerConfiguration;
use core::NeuralNetwork;
use rayon::prelude::*;

mod train;

fn train(
    configuration: &[LayerConfiguration],
    epochs: u32,
    learning_rate: f64,
    momentum: f64,
) -> NeuralNetwork {
    let mut training_data = Vec::new();

    for a in 0..10 {
        for b in 0..10 {
            let sum = (a + b) as f64;
            training_data.push((vec![a as f64, b as f64], vec![sum]));
        }
    }
    
    let mut net = NeuralNetwork::new(configuration);

    net.train(&training_data, learning_rate, momentum, epochs);

    net
}

fn main() {
    let rate = 0.01;
    let momentum = 0.1f64;
    let epochs = 10000;

    // let mut ratings: Mutex<Vec<(f64, Vec<LayerConfiguration>, (u32, f64, f64))>> =
    //     Mutex::new(Vec::new());

    let final_configuration = vec![
        LayerConfiguration::new(2, ActivationFunction::Relu),
        LayerConfiguration::new(5, ActivationFunction::Relu),
        LayerConfiguration::new(1, ActivationFunction::Relu)
    ];

    let trained_nn = train(&final_configuration, epochs, rate, momentum);

    let res = trained_nn.run(&[1f64, 2f64]);

    println!("### {:?}", res);

    // ratings
    //     .lock()
    //     .unwrap()
    //     .push((error, final_configuration, (epochs, rate, momentum)));

    // net_configurations.par_iter().for_each(|(configuration)| {
    //
    // });

    // let mut ratings_lock = ratings.lock().unwrap();
    //
    // ratings_lock
    //     .sort_by(|(error, _, _), (next_error, _, _)| error.partial_cmp(next_error).unwrap());
    //
    // for (index, (error, data, (epochs, rate, momentum))) in ratings_lock.iter().take(10).enumerate()
    // {
    //     println!(
    //         "{}) {:?} - {} (epochs: {}, rate: {}, momentum: {})",
    //         index + 1,
    //         data,
    //         error,
    //         *epochs,
    //         *rate,
    //         *momentum
    //     );
    // }
}
