use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamRelationsRoutes;

impl TeamRelationsRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/relations",
            get(super::TeamRelationsPage::action),
        )
    }
}
