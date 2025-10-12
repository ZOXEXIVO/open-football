use std::future::Future;
use std::pin::Pin;
use std::time::Instant;

pub struct TimeEstimation;

impl TimeEstimation {
    pub fn estimate<T, F: FnOnce() -> T>(action: F) -> (T, u32) {
        let now = Instant::now();

        let result = action();

        (result, now.elapsed().as_millis() as u32)
    }

    pub async fn estimate_async<T, F>(action: F) -> (T, u32)
    where
        F: FnOnce() -> Pin<Box<dyn Future<Output=T> + Send>>,
    {
        let now = Instant::now();

        let result = action().await;

        (result, now.elapsed().as_millis() as u32)
    }
}
