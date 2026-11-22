use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerPersonalRoutes;

impl PlayerPersonalRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/personal",
            get(super::PlayerPersonalPage::action),
        )
    }
}
