use crate::r#match::stores::MatchStore;
use crate::GameAppData;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use core::FootballSimulator;
use core::SimulationResult;
use log::debug;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tokio::task::JoinSet;

#[derive(Deserialize)]
pub struct ProcessQuery {
    pub days: Option<u32>,
}

pub async fn game_process_action(
    State(state): State<GameAppData>,
    Query(query): Query<ProcessQuery>,
) -> impl IntoResponse {
    let days = query.days.unwrap_or(1);

    // If already processing, return immediately
    let process_guard = match Arc::clone(&state.process_lock).try_lock_owned() {
        Ok(guard) => guard,
        Err(_) => return StatusCode::OK,
    };

    let data = Arc::clone(&state.data);
    let i18n = Arc::clone(&state.i18n);
    let cancel_flag = Arc::clone(&state.cancel_flag);

    // Reset cancel flag at start
    cancel_flag.store(false, Ordering::SeqCst);

    // Clone data under read lock (cheap Arc clone), then release lock immediately
    let data_arc = {
        let guard = data.read().await;
        Arc::clone(guard.as_ref().unwrap())
    };

    // Run CPU-bound simulation on the blocking thread pool so tokio worker
    // threads stay free to serve HTTP requests while processing runs.
    // Await completion so the client knows when processing is done.
    let handle = tokio::runtime::Handle::current();
    let _ = tokio::task::spawn_blocking(move || {
        let _guard = process_guard;

        // Deep clone outside any lock
        let mut simulator_data = Arc::unwrap_or_clone(data_arc);
        let mut days_since_sync: u32 = 0;

        for _ in 0..days {
            // Check cancellation before each day
            if cancel_flag.load(Ordering::SeqCst) {
                break;
            }

            let result = handle.block_on(FootballSimulator::simulate(&mut simulator_data));
            if result.has_match_results() && core::is_match_recordings_mode() {
                handle.block_on(write_match_results(result));
            }

            // During multi-day runs (e.g. holiday), publish progress every
            // simulated week so the UI and readers observe intermediate state
            // instead of waiting for the whole range to finish.
            days_since_sync += 1;
            if days_since_sync >= 7 {
                days_since_sync = 0;
                i18n.set_date(simulator_data.date);
                let arc = Arc::new(simulator_data);
                handle.block_on(async {
                    let mut guard = data.write().await;
                    *guard = Some(Arc::clone(&arc));
                });
                simulator_data = Arc::unwrap_or_clone(arc);
            }
        }

        cancel_flag.store(false, Ordering::SeqCst);
        i18n.set_date(simulator_data.date);

        // Write the simulated data back
        handle.block_on(async {
            let mut guard = data.write().await;
            *guard = Some(Arc::new(simulator_data));
        });
    }).await;

    StatusCode::OK
}

#[derive(Serialize)]
pub struct ProcessingStatus {
    pub processing: bool,
}

pub async fn game_processing_status_action(
    State(state): State<GameAppData>,
) -> Json<ProcessingStatus> {
    let processing = state.process_lock.try_lock().is_err();
    Json(ProcessingStatus { processing })
}

pub async fn game_cancel_action(
    State(state): State<GameAppData>,
) -> StatusCode {
    state.cancel_flag.store(true, Ordering::SeqCst);
    StatusCode::OK
}

async fn write_match_results(result: SimulationResult) {
    let now = Instant::now();

    let max_concurrent = core::match_store_max_threads();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));

    let mut tasks = JoinSet::new();

    for match_result in result.match_results {
        if match_result.friendly {
            continue;
        }

        let permit = Arc::clone(&semaphore);
        tasks.spawn(async move {
            let _permit = permit.acquire().await.unwrap();
            MatchStore::store(match_result).await;
        });
    }

    tasks.join_all().await;

    debug!("match results stored in {} ms", now.elapsed().as_millis());
}
