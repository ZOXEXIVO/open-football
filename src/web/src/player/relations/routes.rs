use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerRelationsRoutes;

impl PlayerRelationsRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/relations",
            get(super::PlayerRelationsPage::action),
        )
    }
}
