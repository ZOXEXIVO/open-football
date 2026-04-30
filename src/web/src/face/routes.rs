use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/api/players/{player_id}/face.svg", get(super::face_action))
}
