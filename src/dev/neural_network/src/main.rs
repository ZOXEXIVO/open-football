use std::path::PathBuf;
use crate::model::MyBinaryNet;
use crate::training::{train, TrainingConfig};
use burn::backend::ndarray::NdArrayDevice;
use burn::backend::{Autodiff, NdArray};
use burn::prelude::{Module, Tensor};
use burn::record::{BinBytesRecorder, BinFileRecorder, FullPrecisionSettings, Recorder};

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
    
    let model: MidfielderPassingNeural<MyAutodiffBackend> = train(
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
    
    let path = PathBuf::from_iter(["artifacts", "model.bin"]);

    let recorder = BinFileRecorder::<FullPrecisionSettings>::new();
    model
        .save_file(PathBuf::from_iter(&path), &recorder)
        .expect("Should be able to save the model");


    // // Include the model file as a reference to a byte array
    // static MODEL_BYTES: &[u8] = include_bytes!("../artifacts/model.bin");
    // 
    // // Load model binary record in full precision
    // let record = BinBytesRecorder::<FullPrecisionSettings>::default()
    //     .load(MODEL_BYTES.to_vec(), &device)
    //     .expect("Should be able to load model the model weights from bytes");
    // 
    // // Load that record with the model
    // let new_model = model.load_record(record);

    println!("RESTORED");
}
