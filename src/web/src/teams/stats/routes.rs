use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamStatsRoutes;

impl TeamStatsRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/stats",
            get(super::TeamStatsPage::action),
        )
    }
}
