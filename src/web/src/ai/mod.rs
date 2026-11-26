pub mod routes;

use crate::GameAppData;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Decision-model connection entered from the home-page "AI" badge dialog.
/// `url` is the full endpoint address, used as typed. Held in memory only for
/// the life of the process — the badge renders ON once set, OFF while unset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelSettings {
    pub url: String,
    #[serde(default)]
    pub api_key: String,
}

impl ModelSettings {
    pub fn defaults() -> Self {
        ModelSettings {
            url: "http://localhost:8080/v1/systemone".to_string(),
            api_key: String::new(),
        }
    }

    /// Sends the smallest request the decision endpoint answers — one
    /// two-level score question — and accepts the connection only on 200.
    pub async fn probe(&self) -> Result<(), String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| format!("could not create HTTP client: {e}"))?;
        let body = json!({
            "state": "ping",
            "questions": {
                "ping": {
                    "type": "score",
                    "instructions": "Is the connection up?",
                    "criteria": ["no", "yes"],
                }
            }
        });
        let mut request = http.post(&self.url).json(&body);
        if !self.api_key.is_empty() {
            request = request.bearer_auth(&self.api_key);
        }
        let response = request
            .send()
            .await
            .map_err(|e| format!("endpoint is not reachable: {e}"))?;
        let status = response.status();
        if status == StatusCode::OK {
            return Ok(());
        }
        let text = response.text().await.unwrap_or_default();
        let snippet: String = text.chars().take(200).collect();
        Err(format!("endpoint returned {status}: {snippet}"))
    }
}

/// Cloneable, Arc-backed handle to the process-wide model connection. Mirrors
/// `WorkerRegistry`: every clone shares the same inner lock, so a save
/// from one request is visible to the next page render.
#[derive(Clone, Default)]
pub struct AiConfig {
    inner: Arc<RwLock<Option<ModelSettings>>>,
}

impl AiConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn get(&self) -> Option<ModelSettings> {
        self.inner.read().await.clone()
    }

    pub async fn set(&self, settings: ModelSettings) {
        *self.inner.write().await = Some(settings);
    }

    pub async fn clear(&self) {
        *self.inner.write().await = None;
    }
}

/// Body of the "AI settings" dialog POST.
#[derive(Deserialize)]
pub struct SaveAiRequest {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub api_key: String,
}

impl SaveAiRequest {
    /// The trimmed connection, or `None` when the URL is blank.
    pub fn settings(&self) -> Option<ModelSettings> {
        let url = self.url.trim();
        if url.is_empty() {
            return None;
        }
        Some(ModelSettings {
            url: url.to_string(),
            api_key: self.api_key.trim().to_string(),
        })
    }
}

/// JSON handed back to the dialog after a save attempt.
#[derive(Serialize)]
pub struct SaveAiResult {
    pub status: &'static str,
    pub detail: String,
}

/// Current connection surfaced to the badge/dialog (GET): whether anything is
/// configured plus the values to pre-fill the form with — saved values when
/// present, defaults otherwise.
#[derive(Serialize)]
pub struct AiConfigDto {
    pub configured: bool,
    pub url: String,
    pub api_key: String,
}

pub struct AiApi;

impl AiApi {
    pub async fn config_get(State(state): State<GameAppData>) -> impl IntoResponse {
        let saved = state.ai.get().await;
        let configured = saved.is_some();
        let settings = saved.unwrap_or_else(ModelSettings::defaults);
        Json(AiConfigDto {
            configured,
            url: settings.url,
            api_key: settings.api_key,
        })
    }

    pub async fn config_save(
        State(state): State<GameAppData>,
        Json(body): Json<SaveAiRequest>,
    ) -> impl IntoResponse {
        let Some(settings) = body.settings() else {
            return Json(SaveAiResult {
                status: "error",
                detail: "url is required".to_string(),
            });
        };
        if let Err(detail) = settings.probe().await {
            return Json(SaveAiResult {
                status: "error",
                detail,
            });
        }
        state.ai.set(settings).await;
        Json(SaveAiResult {
            status: "ok",
            detail: String::new(),
        })
    }

    pub async fn config_delete(State(state): State<GameAppData>) -> impl IntoResponse {
        state.ai.clear().await;
        Json(SaveAiResult {
            status: "ok",
            detail: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AiConfig, ModelSettings, SaveAiRequest};
    use std::future::Future;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    // `#[tokio::test]` expands to `::core::future`, and this workspace has its
    // own `core` crate, so the runtime is built by hand.
    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// Answers one HTTP request with `status_line` and hands back the raw
    /// request it received.
    async fn serve_once(status_line: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            while !request_complete(&raw) {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..read]);
            }
            let reply = format!(
                "HTTP/1.1 {status_line}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            );
            socket.write_all(reply.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&raw).into_owned()
        });
        (url, handle)
    }

    fn request_complete(raw: &[u8]) -> bool {
        let text = String::from_utf8_lossy(raw);
        let Some(head_end) = text.find("\r\n\r\n") else {
            return false;
        };
        let length = text[..head_end]
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())?
            })
            .unwrap_or(0);
        raw.len() >= head_end + 4 + length
    }

    fn settings(url: &str, api_key: &str) -> ModelSettings {
        ModelSettings {
            url: url.to_string(),
            api_key: api_key.to_string(),
        }
    }

    #[test]
    fn probe_accepts_a_200_and_sends_the_score_question() {
        block_on(async {
            let (url, server) = serve_once("200 OK").await;
            assert_eq!(settings(&url, "").probe().await, Ok(()));
            let raw = server.await.unwrap();
            assert!(raw.starts_with("POST /v1/systemone "));
            assert!(raw.contains(r#""type":"score""#));
            assert!(!raw.to_ascii_lowercase().contains("authorization:"));
        });
    }

    #[test]
    fn probe_sends_the_api_key_as_a_bearer_token() {
        block_on(async {
            let (url, server) = serve_once("200 OK").await;
            assert_eq!(settings(&url, "secret").probe().await, Ok(()));
            let raw = server.await.unwrap().to_ascii_lowercase();
            assert!(raw.contains("authorization: bearer secret"));
        });
    }

    #[test]
    fn probe_rejects_any_other_status() {
        block_on(async {
            let (url, server) = serve_once("400 Bad Request").await;
            let detail = settings(&url, "").probe().await.unwrap_err();
            assert!(detail.starts_with("endpoint returned 400"), "{detail}");
            server.await.unwrap();
        });
    }

    #[test]
    fn probe_rejects_an_unreachable_endpoint() {
        block_on(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
            drop(listener);
            let detail = settings(&url, "").probe().await.unwrap_err();
            assert!(detail.starts_with("endpoint is not reachable"), "{detail}");
        });
    }

    fn request(url: &str, api_key: &str) -> SaveAiRequest {
        SaveAiRequest {
            url: url.to_string(),
            api_key: api_key.to_string(),
        }
    }

    #[test]
    fn url_without_key_is_accepted_with_an_empty_key() {
        let settings = request("http://localhost:8080/v1/systemone", "").settings();
        assert_eq!(
            settings,
            Some(ModelSettings {
                url: "http://localhost:8080/v1/systemone".to_string(),
                api_key: String::new(),
            })
        );
    }

    #[test]
    fn url_and_key_are_trimmed_and_kept_as_typed() {
        let settings = request("  http://host:9000/v1/systemone/ ", " secret ").settings();
        assert_eq!(
            settings,
            Some(ModelSettings {
                url: "http://host:9000/v1/systemone/".to_string(),
                api_key: "secret".to_string(),
            })
        );
    }

    #[test]
    fn blank_url_is_rejected() {
        assert_eq!(request("   ", "secret").settings(), None);
    }

    #[test]
    fn body_without_url_deserialises_as_blank() {
        let body: SaveAiRequest =
            serde_json::from_str(r#"{"base_url":"http://x","model":"m"}"#).unwrap();
        assert_eq!(body.settings(), None);
    }

    #[test]
    fn defaults_point_at_the_local_systemone_endpoint() {
        let defaults = ModelSettings::defaults();
        assert_eq!(defaults.url, "http://localhost:8080/v1/systemone");
        assert!(defaults.api_key.is_empty());
    }

    #[test]
    fn rejected_save_leaves_the_previous_connection() {
        block_on(async {
            let config = AiConfig::new();
            let saved = request("http://a/v1/systemone", "k").settings().unwrap();
            config.set(saved.clone()).await;

            assert!(request("", "other").settings().is_none());
            assert_eq!(config.get().await, Some(saved));
        });
    }

    #[test]
    fn clear_reports_unconfigured() {
        block_on(async {
            let config = AiConfig::new();
            assert_eq!(config.get().await, None);
            config.set(ModelSettings::defaults()).await;
            config.clear().await;
            assert_eq!(config.get().await, None);
        });
    }
}
