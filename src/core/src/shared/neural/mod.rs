use burn::backend::NdArray;
use burn::backend::ndarray::NdArrayDevice;

pub type DefaultNeuralBackend = NdArray;
pub const DEFAULT_NEURAL_DEVICE: NdArrayDevice = NdArrayDevice::Cpu;