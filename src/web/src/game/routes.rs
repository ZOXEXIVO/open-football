use crate::game::{game_create_action, game_process_action, game_processing_status_action};
use crate::GameAppData;
use axum::routing::{get, post};
use axum::Router;

pub fn game_routes() -> Router<GameAppData> {
    Router::new()
        .route("/api/game/create", get(game_create_action))
        .route("/api/game/process", post(game_process_action))
        .route("/api/game/processing", get(game_processing_status_action))
}
