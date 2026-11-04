use crate::utils::TimeEstimation;
use log::{Level, debug, info, log_enabled};
use std::time::Instant;

const MAX_DURATION_THRESHOLD_MS: u32 = 1000;

pub struct Logging;

impl Logging {
    pub fn estimate<F: FnOnce()>(action: F, message: &str) {
        let (_, duration_ms) = TimeEstimation::estimate(action);

        debug!("{}, {}ms", message, duration_ms);
    }

    pub fn estimate_result_with<T, F: FnOnce() -> T, M: FnOnce(&T, u32) -> String>(
        action: F,
        message_fn: M,
    ) -> T {
        let (result, duration_ms) = TimeEstimation::estimate(action);

        if duration_ms > MAX_DURATION_THRESHOLD_MS {
            info!("{}", message_fn(&result, duration_ms));
        }

        result
    }

    pub async fn estimate_result_async<T, Fut>(action: Fut, message: &str) -> T
    where
        Fut: Future<Output = T>,
    {
        let now = Instant::now();
        let result = action.await;
        let duration_ms = now.elapsed().as_millis() as u32;

        if duration_ms > MAX_DURATION_THRESHOLD_MS {
            info!("{}, {}ms", message, duration_ms);
        }

        result
    }
}

/// A step whose duration is worth a line only when it runs long. The
/// clock is read only while info lines print, and the message is built
/// only for a step that ran long.
pub struct SlowStep(Option<Instant>);

impl SlowStep {
    pub fn start() -> Self {
        SlowStep(log_enabled!(Level::Info).then(Instant::now))
    }

    pub fn finish(self, message: impl FnOnce() -> String) {
        if let Some(started) = self.0 {
            let duration_ms = started.elapsed().as_millis() as u32;
            if duration_ms > MAX_DURATION_THRESHOLD_MS {
                info!("{}, {}ms", message(), duration_ms);
            }
        }
    }
}
