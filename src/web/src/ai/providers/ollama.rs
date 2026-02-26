use log::debug;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use super::AiProvider;
use std::future::Future;
use std::pin::Pin;

pub struct OllamaRequest {
    api: Ollama,
    model: String,
}

impl OllamaRequest {
    pub fn new(host: &str, port: u16, model: &str) -> Self {
        let api = Ollama::new(host, port);
        OllamaRequest { api, model: model.to_string() }
    }
}

impl AiProvider for OllamaRequest {
    fn query(&self, query: String, format: String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send + '_>> {
        Box::pin(async move {
            let prompt = format!("{} | OUTPUT FORMAT: {}", query, format);

            let response = self.api
                .generate(GenerationRequest::new(self.model.clone(), prompt))
                .await;

            match response {
                Ok(ollama_response) => {
                    debug!("Ollama raw response: {}", ollama_response.response);
                    Ok(ollama_response.response)
                },
                Err(e) => Err(format!("Ollama request error: {}", e))
            }
        })
    }
}
