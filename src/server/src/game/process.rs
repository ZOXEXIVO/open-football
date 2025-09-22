use crate::r#match::stores::MatchStore;
use crate::GameAppData;
use axum::extract::State;
use axum::http::{StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use core::FootballSimulator;
use core::SimulationResult;
use log::{debug};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;
use tokio::task::JoinSet;

pub async fn game_process_action(State(mut state): State<GameAppData>) -> impl IntoResponse {
    let data = Arc::clone(&state.data);

    let mut simulator_data_guard = data.write_owned().await;

    if state.match_simulated {
        return (StatusCode::OK, Json(()));
    }

    let result = tokio::task::spawn_blocking(move || {
        let simulator_data = simulator_data_guard.as_mut().unwrap();

        let result = FootballSimulator::simulate(simulator_data);
        if result.has_match_results() {
            state.match_simulated = true;
            
            tokio::task::spawn(async  {
                write_match_results(result).await
            });
        }
    })
    .await;

    if let Ok(_) = result {
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
