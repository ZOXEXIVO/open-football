use crate::utils::TimeEstimation;
use log::{debug, info, warn};

const MAX_DURATION_THRESHOLD_MS: u32 = 100;

pub struct Logging;

impl Logging {
    pub fn estimate<F: FnOnce()>(action: F, message: &str) {
        let (_, duration_ms) = TimeEstimation::estimate(action);

        debug!("{}, {}ms", message, duration_ms);
    }

    pub fn estimate_result<T, F: FnOnce() -> T>(action: F, message: &str) -> T {
        let (result, duration_ms) = TimeEstimation::estimate(action);

        if duration_ms > MAX_DURATION_THRESHOLD_MS {
            warn!("{}, {}ms", message, duration_ms);
        }

        result
    }
}
