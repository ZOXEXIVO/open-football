use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerAwardsRoutes;

impl PlayerAwardsRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/awards",
            get(super::PlayerAwardsPage::action),
        )
    }
}
