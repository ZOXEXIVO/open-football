use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamScoutingRoutes;

impl TeamScoutingRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/scouting",
            get(super::TeamScoutingPage::action),
        )
    }
}
