use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct FaceGetRoutes;

impl FaceGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/api/players/{player_id}/face.svg",
            get(super::FacePortrait::action),
        )
    }
}
