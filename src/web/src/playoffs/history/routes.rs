use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayoffHistoryRoutes;

impl PlayoffHistoryRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/playoffs/{playoff_slug}/history",
            get(super::PlayoffHistoryPage::action),
        )
    }
}
