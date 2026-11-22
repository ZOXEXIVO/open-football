use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamTransfersRoutes;

impl TeamTransfersRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/transfers",
            get(super::TeamTransfersPage::action),
        )
    }
}
