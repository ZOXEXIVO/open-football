use std::sync::OnceLock;

pub trait AIRequest: Send + Sync {
    fn query_ai(&self, query: String, format: String) -> Result<String, String> ;
}

static AI: OnceLock<Box<dyn AIRequest>> = OnceLock::new();

pub fn set_ai(ai: Box<dyn AIRequest>) {
    let _ = AI.set(ai);
}

pub fn ai_instance() -> Option<&'static dyn AIRequest> {
    AI.get().map(|b| b.as_ref())
}

pub fn ai_instance_enabled() -> bool {
    AI.get().is_some()
}
