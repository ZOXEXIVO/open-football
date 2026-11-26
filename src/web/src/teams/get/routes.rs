use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamGetRoutes;

impl TeamGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route("/{lang}/teams/{team_slug}", get(super::TeamGetPage::action))
    }
}
