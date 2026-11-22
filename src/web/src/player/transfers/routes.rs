use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerTransfersRoutes;

impl PlayerTransfersRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/transfers",
            get(super::PlayerTransfersPage::action),
        )
    }
}
