use log::{debug, error};
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use super::AiProvider;
use std::future::Future;
use std::pin::Pin;

pub struct OllamaRequest {
    host: String,
    port: u16,
    model: String,
    batch_size: usize,
    api: Ollama,
}

impl OllamaRequest {
    pub fn new(host: &str, port: u16, model: &str) -> Self {
        let api = Ollama::new(host, port);
        OllamaRequest { host: host.to_string(), port, model: model.to_string(), batch_size: 1, api }
    }

    pub fn with_batch_size(mut self, batch_size: usize) -> Self {
        self.batch_size = batch_size.max(1);
        self
    }
}

impl AiProvider for OllamaRequest {
    fn host(&self) -> &str { &self.host }
    fn port(&self) -> u16 { self.port }
    fn model(&self) -> &str { &self.model }
    fn batch_size(&self) -> usize { self.batch_size }

    fn query(&self, query: String, format: String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + '_>> {
        Box::pin(async move {
            let prompt = format!("{} | OUTPUT FORMAT: {}", query, format);

            let response = self.api
                .generate(GenerationRequest::new(self.model.clone(), prompt))
                .await;

            match response {
                Ok(ollama_response) => {
                    debug!("Ollama response from {}:{} model={}: {} bytes", self.host, self.port, self.model, ollama_response.response.len());
                    Ok(ollama_response.response)
                },
                Err(e) => {
                    debug!("Ollama request failed: host={}:{}, model={}, error={}", self.host, self.port, self.model, e);
                    Err(format!("Ollama request error: {}", e))
                }
            }
        })
    }
}
