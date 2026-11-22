use crate::GameAppData;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;

pub struct GameCreate;

impl GameCreate {
    pub async fn action(State(_state): State<GameAppData>) -> impl IntoResponse {
        StatusCode::OK
    }
}
