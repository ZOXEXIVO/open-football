use std::sync::{Arc, Mutex};
use super::PendingAiRequest;

#[derive(Clone)]
pub struct Ai {
    requests: Arc<Mutex<Vec<PendingAiRequest>>>,
    enabled: bool,
}

impl Ai {
    pub fn new(enabled: bool) -> Self {
        Ai {
            requests: Arc::new(Mutex::new(Vec::new())),
            enabled,
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn push(&self, request: PendingAiRequest) {
        self.requests.lock().unwrap().push(request);
    }

    pub fn drain(&self) -> Vec<PendingAiRequest> {
        self.requests.lock().unwrap().drain(..).collect()
    }
}
