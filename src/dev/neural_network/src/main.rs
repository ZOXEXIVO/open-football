use crate::model::MyBinaryNet;
use crate::training::{train, TrainingConfig};
use burn::backend::ndarray::NdArrayDevice;
use burn::backend::{Autodiff, NdArray};
use burn::prelude::Tensor;

mod model;
mod training;

fn main() {
    type MyBackend = NdArray;
    type MyAutodiffBackend = Autodiff<MyBackend>;

    let device = NdArrayDevice::default();

    let training_data = vec![
        (0f64, 0f64, 1f64),
        (1f64, 0f64, 0f64),
        (0f64, 1f64, 0f64),
        (1f64, 1f64, 0f64),
    ];

    let model: MyBinaryNet<MyAutodiffBackend> = train(
        "artifacts",
        TrainingConfig {
            num_epochs: 3000,
            learning_rate: 1e-2,
            momentum: 1e-2,
            seed: 43,
            batch_size: 1,
        },
        training_data.clone(),
        device,
    );

    for item in training_data {
        let tensor = Tensor::from_data([[item.0, item.1]], &device);
        let result = model.forward(tensor);

        let tensor_data_string = result
            .to_data()
            .iter()
            .map(|x: f32| format!("{:.4}", x))
            .collect::<Vec<String>>()
            .join(", ");

        println!(
            "INPUT: {},{}, RESULT: {:.32}",
            item.0, item.1, tensor_data_string
        );
    }
}
