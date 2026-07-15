pub mod agent;
mod client;
mod jobs;
pub mod routes;
mod tools;

pub use jobs::AiJobs;

use crate::GameAppData;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

/// OpenAI-compatible LLM contract settings entered from the home-page
/// "AI" badge dialog. Held in memory only for the life of the process —
/// the badge renders ON once these are set, OFF while unset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LlmSettings {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

impl LlmSettings {
    /// Values pre-filled into the dialog before anything has been saved —
    /// a local llama.cpp / Ollama-style OpenAI endpoint.
    pub fn defaults() -> Self {
        LlmSettings {
            base_url: "http://192.168.1.71:8080/v1".to_string(),
            model: "unsloth/Qwen3.6-27B-MTP-GGUF:UD-Q8_K_X".to_string(),
            api_key: String::new(),
        }
    }
}

/// Cloneable, Arc-backed handle to the process-wide LLM settings. Mirrors
/// `WorkerRegistry`: every clone shares the same inner lock, so a save
/// from one request is visible to the next page render.
#[derive(Clone, Default)]
pub struct AiConfig {
    inner: Arc<RwLock<Option<LlmSettings>>>,
}

impl AiConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Currently-saved settings, or `None` while the LLM hasn't been
    /// configured yet.
    pub async fn get(&self) -> Option<LlmSettings> {
        self.inner.read().await.clone()
    }

    /// True once valid settings have been saved — drives the ON badge.
    pub async fn is_configured(&self) -> bool {
        self.inner.read().await.is_some()
    }

    pub async fn set(&self, settings: LlmSettings) {
        *self.inner.write().await = Some(settings);
    }
}

/// Body of the "AI settings" dialog POST.
#[derive(Deserialize)]
pub struct SaveAiRequest {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

/// JSON handed back to the dialog after a save attempt.
#[derive(Serialize)]
pub struct SaveAiResult {
    pub status: &'static str,
    pub detail: String,
}

/// Current AI config surfaced to the badge/dialog (GET). Reports whether
/// anything is configured plus the values to pre-fill the form with —
/// saved values when present, hardcoded defaults otherwise.
#[derive(Serialize)]
pub struct AiConfigDto {
    pub configured: bool,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

/// Read the current AI contract so the dialog can pre-fill and the badge
/// can decide ON/OFF without a page render.
pub async fn ai_config_get_action(State(state): State<GameAppData>) -> impl IntoResponse {
    let saved = state.ai.get().await;
    let configured = saved.is_some();
    let settings = saved.unwrap_or_else(LlmSettings::defaults);
    Json(AiConfigDto {
        configured,
        base_url: settings.base_url,
        model: settings.model,
        api_key: settings.api_key,
    })
}

/// Store the LLM contract typed into the dialog in memory. `base_url` and
/// `model` are required; `api_key` is optional (local endpoints rarely
/// need one). Once saved, the home badge flips to ON.
pub async fn ai_config_save_action(
    State(state): State<GameAppData>,
    Json(body): Json<SaveAiRequest>,
) -> impl IntoResponse {
    let base_url = body.base_url.trim().to_string();
    let model = body.model.trim().to_string();
    if base_url.is_empty() || model.is_empty() {
        return Json(SaveAiResult {
            status: "error",
            detail: "base_url and model are required".to_string(),
        });
    }
    state
        .ai
        .set(LlmSettings {
            base_url,
            model,
            api_key: body.api_key.trim().to_string(),
        })
        .await;
    Json(SaveAiResult {
        status: "ok",
        detail: String::new(),
    })
}

/// Long-poll query for a running agent job. `cursor` is the number of tool
/// calls the client has already rendered; the endpoint holds the request
/// until there are more (or the job finishes) so tool activity streams live.
#[derive(Deserialize)]
pub struct ProgressQuery {
    pub job_id: u64,
    #[serde(default)]
    pub cursor: usize,
}

/// Generic progress endpoint shared by every page's AI feature: it only
/// speaks in job ids, so per-page start handlers own their own prompts.
pub async fn ai_progress_action(
    State(state): State<GameAppData>,
    Query(query): Query<ProgressQuery>,
) -> impl IntoResponse {
    match state.ai_jobs.wait(query.job_id, query.cursor).await {
        Some(snapshot) => Json(snapshot).into_response(),
        None => (StatusCode::NOT_FOUND, "unknown job").into_response(),
    }
}
