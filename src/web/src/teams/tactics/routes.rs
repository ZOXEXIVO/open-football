use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamTacticsRoutes;

impl TeamTacticsRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/tactics",
            get(super::TeamTacticsPage::action),
        )
    }
}
