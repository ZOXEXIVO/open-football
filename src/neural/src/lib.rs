#![recursion_limit = "256"]

mod r#match;

use burn::backend::NdArray;
use burn::backend::ndarray::NdArrayDevice;

pub use burn::prelude::*;
pub use r#match::*;

// DEFAULTS

pub type DefaultNeuralBackend = NdArray;

pub const DEFAULT_NEURAL_DEVICE: NdArrayDevice = NdArrayDevice::Cpu;
