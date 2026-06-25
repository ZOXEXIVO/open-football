use crate::GameAppData;
use axum::Router;
use axum::routing::{get, post};

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route("/{lang}/workers", get(super::workers_page_action))
        .route("/api/workers/add", post(super::workers_add_action))
        .route("/api/workers/remove", post(super::workers_remove_action))
        .route("/api/workers/status", get(super::workers_status_action))
}

pub fn workers_routes() -> Router<GameAppData> {
    routes()
}
