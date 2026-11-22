use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerHistoryRoutes;

impl PlayerHistoryRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/history",
            get(super::PlayerHistoryPage::action),
        )
    }
}
