use burn::backend::NdArray;
use burn::backend::ndarray::NdArrayDevice;

pub type DEFAULT_NEURAL_BACKEND = NdArray;
pub const DEFAULT_NEURAL_DEVICE: NdArrayDevice = NdArrayDevice::Cpu;