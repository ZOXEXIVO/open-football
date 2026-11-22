use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamStaffRoutes;

impl TeamStaffRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/staff",
            get(super::TeamStaffPage::action),
        )
    }
}
