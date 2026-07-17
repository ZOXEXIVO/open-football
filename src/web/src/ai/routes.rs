use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route(
            "/api/ai/config",
            get(super::ai_config_get_action)
                .post(super::ai_config_save_action)
                .delete(super::ai_config_delete_action),
        )
        .route("/api/ai/progress", get(super::ai_progress_action))
}

pub fn ai_routes() -> Router<GameAppData> {
    routes()
}
