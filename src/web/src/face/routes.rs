use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route(
            "/api/players/{player_id}/face.jpg",
            get(super::portrait_action),
        )
        .route(
            "/api/players/{player_id}/cutout.png",
            get(super::cutout_action),
        )
}
