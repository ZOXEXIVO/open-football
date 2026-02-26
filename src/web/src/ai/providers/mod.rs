mod ollama;

pub use ollama::OllamaRequest;

use std::future::Future;
use std::pin::Pin;

/// Abstract provider interface. Each AI backend (Ollama, OpenAI, etc.)
/// implements this trait. The registry stores `Box<dyn AiProvider>`.
pub trait AiProvider: Send + Sync {
    fn query(&self, query: String, format: String)
        -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + '_>>;
}
