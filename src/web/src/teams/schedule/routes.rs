use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamScheduleRoutes;

impl TeamScheduleRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/schedule",
            get(super::TeamSchedulePage::action),
        )
    }
}
