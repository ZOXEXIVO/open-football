use crate::GameAppData;
use crate::ai::agent::AiAgent;
use crate::i18n::SUPPORTED_LANGUAGES;
use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The team-report system prompt lives next to this page's handler so each
/// AI-enabled page can own (and edit) its own prompt without touching the
/// shared agent infrastructure in `crate::ai`.
const TEAM_REPORT_PROMPT: &str = include_str!("prompts/team_report.md");

/// Body of the squad-page "AI" button POST.
#[derive(Deserialize)]
pub struct TeamReportRequest {
    pub club_id: u32,
    /// The simulator's currently-enabled UI language code (`en`, `ru`, …);
    /// the report is written in that language.
    #[serde(default)]
    pub lang: String,
}

/// Reply to the start request: a `job_id` to long-poll, or an error the
/// dialog can show immediately.
#[derive(Serialize)]
pub struct TeamReportStart {
    pub job_id: Option<u64>,
    pub error: Option<String>,
}

/// Kick off an AI team report. Registers a job, spawns the agent loop against
/// a cloned world snapshot (so the slow LLM never holds the sim lock), and
/// returns the job id; the dialog then long-polls `/api/ai/progress` to
/// render tool calls live and finally the report text.
pub async fn team_ai_report_action(
    State(state): State<GameAppData>,
    Json(body): Json<TeamReportRequest>,
) -> impl IntoResponse {
    let Some(settings) = state.ai.get().await else {
        return Json(TeamReportStart {
            job_id: None,
            error: Some("AI is not configured".to_string()),
        });
    };

    let data = {
        let guard = state.data.read().await;
        match guard.as_ref() {
            Some(data) => Arc::clone(data),
            None => {
                return Json(TeamReportStart {
                    job_id: None,
                    error: Some("Simulator data not loaded".to_string()),
                });
            }
        }
    };

    let club_name = data
        .club(body.club_id)
        .map(|club| club.name.clone())
        .unwrap_or_default();
    let language = SUPPORTED_LANGUAGES
        .iter()
        .find(|(code, _, _)| *code == body.lang)
        .map(|(_, _, name)| *name)
        .unwrap_or("English");

    let system = format!(
        "{TEAM_REPORT_PROMPT}\n\n## Response language\nWrite your entire final report in {language}."
    );
    let task = format!(
        "Produce a report on the team belonging to the club with id {} (\"{}\").",
        body.club_id, club_name
    );

    let handle = state.ai_jobs.create();
    let job_id = handle.id();
    let agent = AiAgent::new(settings, data);
    tokio::spawn(async move {
        agent.run(&system, &task, &handle).await;
    });

    Json(TeamReportStart {
        job_id: Some(job_id),
        error: None,
    })
}
