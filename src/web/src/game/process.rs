use crate::r#match::stores::MatchStore;
use crate::GameAppData;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use core::FootballSimulator;
use core::SimulationResult;
use log::debug;
use std::sync::Arc;
use std::time::Instant;
use tokio::task::JoinSet;

pub async fn game_process_action(State(state): State<GameAppData>) -> impl IntoResponse {
    let data = Arc::clone(&state.data);

    let mut simulator_data_guard = data.write_owned().await;

    let result = tokio::task::spawn_blocking(move || {
        let simulator_data = simulator_data_guard.as_mut().unwrap();

        if state.is_one_shot_game && simulator_data.match_played {
            return;
        }

        let result = FootballSimulator::simulate(simulator_data);
        if result.has_match_results() {
            tokio::task::spawn(async {
                write_match_results(result).await
            });

            simulator_data.match_played = true;
        }
    })
    .await;

    let mut headers = HeaderMap::new();
    headers.insert("HX-Refresh", HeaderValue::from_static("true"));

    if result.is_ok() {
        (StatusCode::OK, headers, "")
    } else {
        (StatusCode::BAD_REQUEST, headers, "")
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
