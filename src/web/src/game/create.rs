use crate::GameAppData;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use core::utils::TimeEstimation;
use database::DatabaseGenerator;

pub async fn game_create_action(State(state): State<GameAppData>) -> impl IntoResponse {
    let mut state_data = state.data.write().await;

    let cloned_state = GameAppData::clone(&state);

    let generation_result = tokio::task::spawn_blocking(move || {
        let (generated_data, estimated) =
            TimeEstimation::estimate(|| DatabaseGenerator::generate(&cloned_state.database));

        (generated_data, estimated)
    })
    .await;

    let (data, estimated) = generation_result.unwrap();

    *state_data = Some(data);

    let mut headers = HeaderMap::new();

    headers.insert("Location", "/".parse().unwrap());
    headers.insert("Estimated", estimated.to_string().parse().unwrap());

    (StatusCode::OK, headers, Json(()))
}
