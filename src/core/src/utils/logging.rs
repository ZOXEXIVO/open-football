use crate::utils::TimeEstimation;
use log::{debug, info};

const MAX_DURATION_THRESHOLD_MS: u32 = 1000;

pub struct Logging;

impl Logging {
    pub fn estimate<F: FnOnce()>(action: F, message: &str) {
        let (_, duration_ms) = TimeEstimation::estimate(action);

        debug!("{}, {}ms", message, duration_ms);
    }

    pub fn estimate_result<T, F: FnOnce() -> T>(action: F, message: &str) -> T {
        let (result, duration_ms) = TimeEstimation::estimate(action);

        if duration_ms > MAX_DURATION_THRESHOLD_MS {
            info!("{}, {}ms", message, duration_ms);
        }

        result
    }

    pub async fn estimate_result_async<T, Fut>(action: Fut, message: &str) -> T
    where
        Fut: Future<Output = T>,
    {
        let now = std::time::Instant::now();
        let result = action.await;
        let duration_ms = now.elapsed().as_millis() as u32;

        if duration_ms > MAX_DURATION_THRESHOLD_MS {
            info!("{}, {}ms", message, duration_ms);
        }

        result
    }
}
