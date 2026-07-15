use crate::GameAppData;
use crate::ai::agent::AiAgent;
use crate::i18n::SUPPORTED_LANGUAGES;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The player-dossier system prompt lives next to this page's handler so the
/// player page owns (and can edit) its own prompt, independent of the team
/// report and the shared agent infrastructure in `crate::ai`.
const PLAYER_REPORT_PROMPT: &str = include_str!("prompts/player_report.md");

/// Body of the player-page "AI" button POST.
#[derive(Deserialize)]
pub struct PlayerReportRequest {
    pub player_id: u32,
    /// The simulator's currently-enabled UI language code (`en`, `ru`, …);
    /// the dossier is written in that language.
    #[serde(default)]
    pub lang: String,
}

/// Reply to the start request: a `job_id` to long-poll, or an error.
#[derive(Serialize)]
pub struct PlayerReportStart {
    pub job_id: Option<u64>,
    pub error: Option<String>,
}

/// Kick off an AI player dossier. Registers a job, spawns the agent loop
/// against a cloned world snapshot (so the slow LLM never holds the sim
/// lock), and returns the job id; the dialog then long-polls
/// `/api/ai/progress` to render tool calls live and finally the dossier.
pub async fn player_ai_report_action(
    State(state): State<GameAppData>,
    Json(body): Json<PlayerReportRequest>,
) -> impl IntoResponse {
    let Some(settings) = state.ai.get().await else {
        return Json(PlayerReportStart {
            job_id: None,
            error: Some("AI is not configured".to_string()),
        });
    };

    let data = {
        let guard = state.data.read().await;
        match guard.as_ref() {
            Some(data) => Arc::clone(data),
            None => {
                return Json(PlayerReportStart {
                    job_id: None,
                    error: Some("Simulator data not loaded".to_string()),
                });
            }
        }
    };

    let player_name = data
        .player(body.player_id)
        .map(|player| player.full_name.to_string())
        .unwrap_or_default();
    let language = SUPPORTED_LANGUAGES
        .iter()
        .find(|(code, _, _)| *code == body.lang)
        .map(|(_, _, name)| *name)
        .unwrap_or("English");

    let system = format!(
        "{PLAYER_REPORT_PROMPT}\n\n## Response language\nWrite your entire final dossier in {language}."
    );
    let task = format!(
        "Produce a scouting dossier on the player with id {} (\"{}\").",
        body.player_id, player_name
    );

    let handle = state.ai_jobs.create();
    let job_id = handle.id();
    let agent = AiAgent::new(settings, data);
    tokio::spawn(async move {
        agent.run(&system, &task, &handle).await;
    });

    Json(PlayerReportStart {
        job_id: Some(job_id),
        error: None,
    })
}
