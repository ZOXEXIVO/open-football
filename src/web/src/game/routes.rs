use crate::GameAppData;
use crate::game::{
    game_cancel_action, game_create_action, game_process_action, game_processing_status_action,
};
use axum::Router;
use axum::routing::{get, post};

pub fn game_routes() -> Router<GameAppData> {
    Router::new()
        .route("/api/game/create", get(game_create_action))
        .route("/api/game/process", post(game_process_action))
        .route("/api/game/processing", get(game_processing_status_action))
        .route("/api/game/cancel", post(game_cancel_action))
}
