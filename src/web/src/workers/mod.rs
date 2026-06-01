pub mod routes;

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND, CPU_CORES, CSS_VERSION};
use crate::views::{self, MenuSection};
use crate::worker::{WorkerSnapshot, WorkerStatus};
use crate::{ApiResult, GameAppData, I18n};
use askama::Template;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use core::MatchRuntime;
use serde::Deserialize;

#[derive(Deserialize)]
pub struct WorkersPageRequest {
    pub lang: String,
}

#[derive(Template, askama_web::WebTemplate)]
#[template(path = "workers/index.html")]
pub struct WorkersPageTemplate {
    pub css_version: &'static str,
    pub computer_name: &'static str,
    pub cpu_brand: &'static str,
    pub cores_count: usize,
    pub i18n: I18n,
    pub lang: String,
    pub title: String,
    pub sub_title_prefix: String,
    pub sub_title_suffix: String,
    pub sub_title: String,
    pub sub_title_link: String,
    pub sub_title_country_code: String,
    pub header_color: String,
    pub foreground_color: String,
    pub menu_sections: Vec<MenuSection>,

    pub total: usize,
    pub ready: usize,
    pub total_threads: usize,
    pub total_batches: u64,
    pub total_matches: u64,
    pub total_failures: u64,
    /// True when no remote workers are configured. The table still
    /// renders the local row, but a side note explains how to add
    /// remote workers.
    pub no_remote_workers: bool,

    pub workers: Vec<WorkerRowDto>,
}

/// Render-friendly per-worker row. Mirrors `WorkerSnapshot` with the
/// status flattened into a label + detail pair the template can drop
/// into a chip without further branching.
pub struct WorkerRowDto {
    pub address: String,
    pub computer_name: String,
    pub cpu_brand: String,
    pub threads: usize,
    pub version: String,
    pub status_label: &'static str,
    pub status_detail: String,
    pub batches_sent: u64,
    pub matches_completed: u64,
    pub failures: u64,
    pub last_latency_ms: Option<u64>,
    /// Pretty-printed matches-per-second EWMA from the registry,
    /// rounded for display. `None` until the worker has completed at
    /// least one batch — the dispatcher seeds it from thread count
    /// for that first dispatch.
    pub throughput_mps: Option<u64>,
    pub last_error: Option<String>,
}

impl WorkerRowDto {
    fn from_snapshot(w: WorkerSnapshot) -> Self {
        let throughput_mps = w.throughput_mps().map(|v| v.round() as u64);
        let (status_label, status_detail) = match &w.status {
            WorkerStatus::Connecting => ("connecting", String::new()),
            WorkerStatus::Ready => ("ready", String::new()),
            WorkerStatus::VersionMismatch { worker_version } => {
                ("version_mismatch", worker_version.clone())
            }
            WorkerStatus::Unreachable { reason } => ("unreachable", reason.clone()),
        };
        WorkerRowDto {
            address: w.address,
            computer_name: w.computer_name,
            cpu_brand: w.cpu_brand,
            threads: w.threads,
            version: w.version,
            status_label,
            status_detail,
            batches_sent: w.stats.batches_sent,
            matches_completed: w.stats.matches_completed,
            failures: w.stats.failures,
            last_latency_ms: w.stats.last_latency_ms,
            throughput_mps,
            last_error: w.stats.last_error,
        }
    }

    /// Synthetic row for the coordinator's own in-process rayon pool —
    /// the dispatcher's virtual `Local` slot. Always rendered first so
    /// the operator can see the local machine's capacity alongside
    /// remote workers. Per-batch counters (batches/matches/failures
    /// /last_latency) aren't tracked for the local slot, only its
    /// EWMA throughput is.
    fn local_row(throughput_mps: Option<u64>) -> Self {
        WorkerRowDto {
            address: "in-process".to_string(),
            computer_name: COMPUTER_NAME.clone(),
            cpu_brand: CPU_BRAND.clone(),
            threads: MatchRuntime::engine_pool().num_threads(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            status_label: "local",
            status_detail: String::new(),
            batches_sent: 0,
            matches_completed: 0,
            failures: 0,
            last_latency_ms: None,
            throughput_mps,
            last_error: None,
        }
    }
}

pub async fn workers_page_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<WorkersPageRequest>,
) -> ApiResult<impl IntoResponse> {
    let i18n = state.i18n.for_lang(&route_params.lang);
    let current_path = format!("/{}/workers", &route_params.lang);
    let menu_sections = views::search_menu(&i18n, &route_params.lang, &current_path);

    let snapshot = state.workers.snapshot().await;
    let local_throughput_mpms = state.workers.local_throughput().await;

    let total = snapshot.len();
    let mut ready = 0usize;
    let mut total_threads = 0usize;
    let mut total_batches = 0u64;
    let mut total_matches = 0u64;
    let mut total_failures = 0u64;
    for w in &snapshot {
        if matches!(w.status, WorkerStatus::Ready) {
            ready += 1;
        }
        total_threads += w.threads;
        total_batches = total_batches.saturating_add(w.stats.batches_sent);
        total_matches = total_matches.saturating_add(w.stats.matches_completed);
        total_failures = total_failures.saturating_add(w.stats.failures);
    }

    let local_throughput_mps = local_throughput_mpms.map(|m| (m * 1000.0).round() as u64);
    let local_row = WorkerRowDto::local_row(local_throughput_mps);
    total_threads += local_row.threads;

    let mut workers: Vec<WorkerRowDto> = Vec::with_capacity(snapshot.len() + 1);
    workers.push(local_row);
    workers.extend(snapshot.into_iter().map(WorkerRowDto::from_snapshot));

    Ok(WorkersPageTemplate {
        css_version: CSS_VERSION,
        computer_name: &COMPUTER_NAME,
        cpu_brand: &CPU_BRAND,
        cores_count: *CPU_CORES,
        i18n,
        lang: route_params.lang.clone(),
        title: "Workers".to_string(),
        sub_title_prefix: String::new(),
        sub_title_suffix: String::new(),
        sub_title: String::new(),
        sub_title_link: String::new(),
        sub_title_country_code: String::new(),
        header_color: String::new(),
        foreground_color: String::new(),
        menu_sections,
        total,
        ready,
        total_threads,
        total_batches,
        total_matches,
        total_failures,
        no_remote_workers: total == 0,
        workers,
    })
}
