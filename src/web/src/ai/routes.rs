use crate::GameAppData;
use crate::ai::AiApi;
use axum::Router;
use axum::routing::get;

pub struct AiRoutes;

impl AiRoutes {
    pub fn routes() -> Router<GameAppData> {
        Router::new()
            .route(
                "/api/ai/config",
                get(AiApi::config_get)
                    .post(AiApi::config_save)
                    .delete(AiApi::config_delete),
            )
            .route("/api/ai/progress", get(AiApi::progress))
    }
}
