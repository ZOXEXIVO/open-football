use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerMatchesRoutes;

impl PlayerMatchesRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/matches",
            get(super::PlayerMatchesPage::action),
        )
    }
}
