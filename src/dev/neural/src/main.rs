#![recursion_limit = "256"]

use burn::backend::{Autodiff, Wgpu};
use burn::config::Config;
use burn::data::dataloader::batcher::Batcher;
use burn::data::dataloader::DataLoaderBuilder;
use burn::data::dataset::InMemDataset;
use burn::nn::loss::MseLoss;
use burn::nn::loss::Reduction::Mean;
use burn::optim::AdamConfig;
use burn::prelude::{Backend, Module, Tensor};
use burn::record::{BinFileRecorder, FullPrecisionSettings};
use burn::tensor::backend::AutodiffBackend;
use burn::train::metric::LossMetric;
use burn::train::{LearnerBuilder, RegressionOutput, TrainOutput, TrainStep, ValidStep};
use neural::{MidfielderPassingNeural, MidfielderPassingNeuralConfig};
use std::path::PathBuf;
use burn::backend::wgpu::WgpuDevice;

type NeuralNetworkDevice = WgpuDevice;
type NeuralNetworkBackend = Wgpu;
type NeuralNetworkAutodiffBackend = Autodiff<NeuralNetworkBackend>;

type NeuralNetwork<B> = MidfielderPassingNeural<B>;
type NeuralNetworkConfig = MidfielderPassingNeuralConfig;
type NeuralNetworkAutoDiff = MidfielderPassingNeural<NeuralNetworkAutodiffBackend>;

fn main() {
    let device = NeuralNetworkDevice::default();

    let training_data = vec![
        (70f64, 0f64, 88.0f64),
        (70f64, 0f64, 137.0f64),
        (32f64, 4f64, 137.0f64),
        (22f64, 8f64, 137.0f64),
        (77f64, 3f64, 555.0f64),
        (30f64, 1f64, 137.0f64),
        (87f64, 6f64, 111.0f64)
    ];

    let training_additional_data = vec![
        // (4f64, 4f64, 0f64),
        // (20f64, 8f64, 0f64),
        // (44f64, 4f64, 0f64),
        // (19f64, 4f64, 0f64),
        // (90f64, 2f64, 0f64),
    ];

    let model: NeuralNetworkAutoDiff = train::<NeuralNetworkAutodiffBackend>(
        "artifacts",
        TrainingConfig {
            num_epochs: 15000,
            learning_rate: 1e-3,
            momentum: 1e-2,
            seed: 43,
            batch_size: 1,
        },
        training_data.clone(),
        device.clone(),
    );

    for item in training_data.iter().chain(&training_additional_data) {
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
}

#[derive(Config)]
pub struct TrainingConfig {
    #[config(default = 1000)]
    pub num_epochs: usize,

    #[config(default = 1e-3)]
    pub learning_rate: f64,

    #[config(default = 1e-2)]
    pub momentum: f64,

    #[config(default = 42)]
    pub seed: u64,

    #[config(default = 2)]
    pub batch_size: usize,
}

#[derive(Debug, Clone)]
struct BinaryDataBatcher<B: Backend> {
    device: B::Device,
}

impl<B: Backend> BinaryDataBatcher<B> {
    pub fn new(device: B::Device) -> Self {
        BinaryDataBatcher {
            device: device.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrainingBatch<B: Backend> {
    pub inputs: Tensor<B, 2>,
    pub targets: Tensor<B, 1>,
}

type BatcherItem = (f64, f64, f64);

impl<B: Backend> Batcher<B, BatcherItem, TrainingBatch<B>> for BinaryDataBatcher<B> {
    fn batch(&self, items: Vec<BatcherItem>, _device: &B::Device) -> TrainingBatch<B> {
        let mut inputs: Vec<Tensor<B, 2>> = Vec::new();

        for item in items.iter() {
            inputs.push(Tensor::from_floats([[item.0, item.1]], &self.device))
        }

        let inputs = Tensor::cat(inputs, 0);

        let targets = items
            .iter()
            .map(|item| Tensor::<B, 1>::from_floats([item.2], &self.device))
            .collect();

        let targets = Tensor::cat(targets, 0);

        TrainingBatch { inputs, targets }
    }
}

fn create_artifact_dir(artifact_dir: &str) {
    std::fs::remove_dir_all(artifact_dir).ok();
    std::fs::create_dir_all(artifact_dir).ok();
}

impl<B: AutodiffBackend> TrainStep<TrainingBatch<B>, RegressionOutput<B>> for NeuralNetwork<B> {
    fn step(&self, item: TrainingBatch<B>) -> TrainOutput<RegressionOutput<B>> {
        let output = self.forward_step(item);

        TrainOutput::new(self, output.loss.backward(), output)
    }
}

impl<B: Backend> ValidStep<TrainingBatch<B>, RegressionOutput<B>> for NeuralNetwork<B> {
    fn step(&self, item: TrainingBatch<B>) -> RegressionOutput<B> {
        self.forward_step(item)
    }
}
pub fn train<B: AutodiffBackend>(
    artifact_dir: &str,
    config: TrainingConfig,
    training_data: Vec<(f64, f64, f64)>,
    device: B::Device,
) -> NeuralNetwork<B> {
    create_artifact_dir(artifact_dir);

    config
        .save(format!("{artifact_dir}/config.json"))
        .expect("Config should be saved successfully");

    B::seed(config.seed);

    let model: NeuralNetwork<B> = NeuralNetworkConfig::init(&device);

    let optimizer = AdamConfig::new().init();

    let train_dataset = InMemDataset::new(training_data.clone());

    let train_data = DataLoaderBuilder::new(BinaryDataBatcher::new(device.clone()))
        .batch_size(1)
        .build(train_dataset);

    let valid_dataset = InMemDataset::new(training_data);

    let valid_data = DataLoaderBuilder::new(BinaryDataBatcher::new(device.clone()))
        .batch_size(1)
        .build(valid_dataset);

    let learner = LearnerBuilder::new(artifact_dir)
        .metric_train_numeric(LossMetric::new())
        .metric_valid_numeric(LossMetric::new())
        //.with_file_checkpointer(CompactRecorder::new())
        .devices(vec![device.clone()])
        .num_epochs(config.num_epochs)
        .summary()
        .build(model, optimizer, config.learning_rate);

    learner.fit(train_data, valid_data)
}

trait NeuralTrait<B: Backend> {
    fn forward_step(&self, item: TrainingBatch<B>) -> RegressionOutput<B>;
}

impl<B: Backend> NeuralTrait<B> for NeuralNetwork<B> {
    fn forward_step(&self, item: TrainingBatch<B>) -> RegressionOutput<B> {
        let targets: Tensor<B, 2> = item.targets.unsqueeze_dim(1);
        let output: Tensor<B, 2> = self.forward(item.inputs);

        let loss = MseLoss::new().forward(output.clone(), targets.clone(), Mean);

        RegressionOutput {
            loss,
            output,
            targets,
        }
    }
}
