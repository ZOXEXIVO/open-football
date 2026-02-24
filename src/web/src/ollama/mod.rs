use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use core::AIRequest;

pub struct OllamaRequest {
    api: Ollama,
    model: String
}

impl OllamaRequest {
    pub fn from_env() -> Self {
        let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://localhost".to_string());
        let port: u16 = std::env::var("OLLAMA_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(11434);
        let model = std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gpt-oss:20b".to_string());

        let api = Ollama::new(host, port);

        OllamaRequest { api, model }
    }
}

impl AIRequest for OllamaRequest {
    fn query_ai(&self, query: String, format: String) -> Result<String, String> {
        let prompt = format!("{} | OUTPUT FORMAT: {}", query, format);

        let response = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(
                self.api.generate(GenerationRequest::new(self.model.clone(), prompt))
            )
        });

        match response {
            Ok(ollama_response) => Ok(ollama_response.response),
            Err(e) => Err(format!("Ollama request error: {}", e))
        }
    }
}
