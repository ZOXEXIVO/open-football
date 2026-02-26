use crate::GameAppData;
use axum::routing::{delete, get, post};
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route("/{lang}/ai", get(super::ai_page_action))
        .route("/api/ai/providers", post(super::ai_add_provider_action))
        .route("/api/ai/providers/{provider_id}", delete(super::ai_remove_provider_action))
}
