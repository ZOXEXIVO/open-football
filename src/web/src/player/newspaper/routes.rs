use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerNewspaperRoutes;

impl PlayerNewspaperRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/newspaper",
            get(super::PlayerNewspaperPage::action),
        )
    }
}
