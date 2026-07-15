use crate::GameAppData;
use axum::Router;
use axum::routing::{get, post};

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route(
            "/{lang}/players/{player_slug}",
            get(super::player_get_action),
        )
        .route(
            "/api/ai/player-report",
            post(super::ai_report::player_ai_report_action),
        )
}
