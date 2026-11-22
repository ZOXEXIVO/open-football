use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayoffGetRoutes;

impl PlayoffGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/playoffs/{playoff_slug}",
            get(super::PlayoffGetPage::action),
        )
    }
}
