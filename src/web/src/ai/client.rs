use crate::ai::LlmSettings;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

/// One tool advertised to the model: an OpenAI function-tool schema.
#[derive(Clone, Serialize)]
pub struct ToolSchema {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: Value,
}

/// A tool call the model asked us to run, parsed from a chat response.
#[derive(Clone, Debug, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    pub function: ToolFunction,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ToolFunction {
    pub name: String,
    #[serde(default)]
    pub arguments: String,
}

/// Result of a single chat round: the assistant text (if any) plus any
/// tool calls it wants executed before continuing.
pub struct ChatTurn {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Deserialize)]
struct ChatResponse {
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCall>>,
}

/// Thin OpenAI-compatible chat-completions client driven by the operator's
/// in-memory `LlmSettings`. One `chat()` call is one round-trip; the agent
/// loop lives in `AiAgent`.
pub struct AiClient {
    http: reqwest::Client,
    settings: LlmSettings,
}

impl AiClient {
    pub fn new(settings: LlmSettings) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        AiClient { http, settings }
    }

    fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.settings.base_url.trim_end_matches('/'))
    }

    /// Send the running message list (plus tool schemas) and return the
    /// assistant's reply. Errors carry a short human-readable reason for
    /// the dialog.
    pub async fn chat(&self, messages: &[Value], tools: &[ToolSchema]) -> Result<ChatTurn, String> {
        let mut body = json!({
            "model": self.settings.model,
            "messages": messages,
            "temperature": 0.6,
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::to_value(tools).unwrap_or(Value::Null);
            body["tool_choice"] = json!("auto");
        }

        let mut request = self.http.post(self.endpoint()).json(&body);
        if !self.settings.api_key.is_empty() {
            request = request.bearer_auth(&self.settings.api_key);
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            let snippet: String = text.chars().take(300).collect();
            return Err(format!("LLM returned {status}: {snippet}"));
        }

        let parsed: ChatResponse = response
            .json()
            .await
            .map_err(|e| format!("could not parse LLM response: {e}"))?;

        let message = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| "LLM response had no choices".to_string())?
            .message;

        Ok(ChatTurn {
            content: message.content,
            tool_calls: message.tool_calls.unwrap_or_default(),
        })
    }
}
