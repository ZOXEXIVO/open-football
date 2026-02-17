use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/api/players/{player_id}/face.svg",
        get(super::face_action),
    )
}
