use crate::GameAppData;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

pub async fn game_create_action(State(_state): State<GameAppData>) -> impl IntoResponse {
    StatusCode::OK
}
