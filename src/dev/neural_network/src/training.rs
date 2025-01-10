use crate::model::{MyBinaryNet, MyBinaryNetConfig};
use burn::data::dataloader::batcher::Batcher;
use burn::data::dataloader::DataLoaderBuilder;
use burn::data::dataset::InMemDataset;
use burn::optim::AdamConfig;
use burn::prelude::*;
use burn::record::CompactRecorder;
use burn::tensor::backend::AutodiffBackend;
use burn::tensor::Tensor;
use burn::train::metric::LossMetric;
use burn::train::{LearnerBuilder, RegressionOutput, TrainOutput, TrainStep, ValidStep};

#[derive(Config)]
pub struct TrainingConfig {
    #[config(default = 1000)]
    pub num_epochs: usize,

    #[config(default = 1e-2)]
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

impl<B: Backend> Batcher<BatcherItem, TrainingBatch<B>> for BinaryDataBatcher<B> {
    fn batch(&self, items: Vec<BatcherItem>) -> TrainingBatch<B> {
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
    // Remove existing artifacts before to get an accurate learner summary
    std::fs::remove_dir_all(artifact_dir).ok();
    std::fs::create_dir_all(artifact_dir).ok();
}

impl<B: AutodiffBackend> TrainStep<TrainingBatch<B>, RegressionOutput<B>> for MyBinaryNet<B> {
    fn step(&self, item: TrainingBatch<B>) -> TrainOutput<RegressionOutput<B>> {
        let output = self.forward_step(item);

        TrainOutput::new(self, output.loss.backward(), output)
    }
}

impl<B: Backend> ValidStep<TrainingBatch<B>, RegressionOutput<B>> for MyBinaryNet<B> {
    fn step(&self, item: TrainingBatch<B>) -> RegressionOutput<B> {
        self.forward_step(item)
    }
}

pub fn train<B: AutodiffBackend>(
    artifact_dir: &str,
    config: TrainingConfig,
    training_data: Vec<(f64, f64, f64)>,
    device: B::Device,
) -> MyBinaryNet<B> {
    create_artifact_dir(artifact_dir);

    config
        .save(format!("{artifact_dir}/config.json"))
        .expect("Config should be saved successfully");

    B::seed(config.seed);

    let model: MyBinaryNet<B> = MyBinaryNetConfig::init(&device);

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
        .with_file_checkpointer(CompactRecorder::new())
        .devices(vec![device.clone()])
        .num_epochs(config.num_epochs)
        .summary()
        .build(model, optimizer, config.learning_rate);

    learner.fit(train_data, valid_data)
}
