mod batch;
mod request;

pub use batch::*;
pub use request::*;

use std::future::Future;
use std::pin::Pin;
use std::sync::OnceLock;

/// Core AI service trait. Web crate provides the implementation.
/// Core only defines the interface — no runtime dependencies.
pub trait AiService: Send + Sync {
    /// Whether any AI provider is available.
    fn is_enabled(&self) -> bool;

    /// Execute a batch of pending requests concurrently across providers.
    /// Returns a future — core `.await`s it; the runtime lives in web.
    fn execute_batch(
        &self,
        requests: Vec<PendingAiRequest>,
    ) -> Pin<Box<dyn Future<Output = Vec<CompletedAiRequest>> + Send + '_>>;
}

static AI_SERVICE: OnceLock<Box<dyn AiService>> = OnceLock::new();

/// Process-global registry holding the single [`AiService`] implementation
/// the web crate installs at startup. Core reads it back through
/// [`AiServiceRegistry::get`] so the engine stays runtime-agnostic.
pub struct AiServiceRegistry;

impl AiServiceRegistry {
    /// Install the service. Idempotent — the first call wins; later calls
    /// are ignored (the underlying `OnceLock` is write-once).
    pub fn set(service: Box<dyn AiService>) {
        let _ = AI_SERVICE.set(service);
    }

    /// Borrow the installed service, if any.
    pub fn get() -> Option<&'static dyn AiService> {
        AI_SERVICE.get().map(|b| b.as_ref())
    }
}
