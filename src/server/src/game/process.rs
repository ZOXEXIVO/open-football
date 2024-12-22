use crate::stores::MatchStore;
use crate::GameAppData;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use core::utils::TimeEstimation;
use core::FootballSimulator;
use core::SimulationResult;
use futures::stream::{FuturesUnordered, StreamExt};
use log::{debug, info};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::AsyncWriteExt;
use tokio::stream;
use tokio::task::JoinSet;

pub async fn game_process_action(State(state): State<GameAppData>) -> impl IntoResponse {
    let data = Arc::clone(&state.data);

    let mut simulator_data_guard = data.write_owned().await;

    let result = tokio::task::spawn_blocking(move || {
        let simulator_data = simulator_data_guard.as_mut().unwrap();
        FootballSimulator::simulate(simulator_data)
    })
    .await;

    if let Ok(res) = result {
        if res.has_match_results() {
            write_match_results(res).await;
        }

        (StatusCode::OK, Json(()))
    } else {
        (StatusCode::BAD_REQUEST, Json(()))
    }
}

async fn write_match_results(result: SimulationResult) {
    let mut tasks = JoinSet::new();

    for match_result in result.match_results {
        tasks.spawn(MatchStore::store(match_result));
    }

    let now = Instant::now();

    tasks.join_all().await;

    let elapsed = now.elapsed().as_millis();

    debug!("match results stored in {} ms", elapsed);
}
