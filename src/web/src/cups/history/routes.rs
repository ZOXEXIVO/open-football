use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CupHistoryRoutes;

impl CupHistoryRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/cups/{cup_slug}/history",
            get(super::CupHistoryPage::action),
        )
    }
}
