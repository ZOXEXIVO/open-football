use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamFinancesRoutes;

impl TeamFinancesRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/finances",
            get(super::TeamFinancesPage::action),
        )
    }
}
