use super::{AiServiceRegistry, CompletedAiRequest, PendingAiRequest};

pub struct AiBatchProcessor;

impl AiBatchProcessor {
    /// Delegate batch execution to the registered AiService implementation.
    pub async fn execute(requests: Vec<PendingAiRequest>) -> Vec<CompletedAiRequest> {
        let service = match AiServiceRegistry::get() {
            Some(s) => s,
            None => return Vec::new(),
        };

        service.execute_batch(requests).await
    }
}
