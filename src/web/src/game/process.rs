use crate::r#match::stores::MatchStore;
use crate::GameAppData;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use core::FootballSimulator;
use core::SimulationResult;
use log::debug;
use serde::Deserialize;
use std::sync::Arc;
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
    let data = Arc::clone(&state.data);
    let days = query.days.unwrap_or(1);

    let mut simulator_data_guard = data.write_owned().await;
    let simulator_data = simulator_data_guard.as_mut().unwrap();

    for _ in 0..days {
        let result = FootballSimulator::simulate(simulator_data).await;
        if result.has_match_results() {
            if core::is_match_recordings_mode() {
                tokio::task::spawn(async {
                    write_match_results(result).await
                });
            }

            simulator_data.match_played = true;
        }
    }

    let mut headers = HeaderMap::new();
    headers.insert("HX-Refresh", HeaderValue::from_static("true"));

    (StatusCode::OK, headers, "")
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
