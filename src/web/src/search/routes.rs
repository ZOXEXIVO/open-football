use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct SearchGetRoutes;

impl SearchGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .route("/{lang}/search", get(super::SearchPage::action))
            .route("/api/search", get(super::SearchApi::action))
    }
}
