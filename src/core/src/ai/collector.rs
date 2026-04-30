use super::PendingAiRequest;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Ai {
    requests: Arc<Mutex<Vec<PendingAiRequest>>>,
}

impl Default for Ai {
    fn default() -> Self {
        Self::new()
    }
}

impl Ai {
    pub fn new() -> Self {
        Ai {
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn push(&self, request: PendingAiRequest) {
        self.requests.lock().unwrap().push(request);
    }

    pub fn drain(&self) -> Vec<PendingAiRequest> {
        self.requests.lock().unwrap().drain(..).collect()
    }
}
