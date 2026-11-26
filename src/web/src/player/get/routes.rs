use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerGetRoutes;

impl PlayerGetRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}",
            get(super::PlayerGetPage::action),
        )
    }
}
