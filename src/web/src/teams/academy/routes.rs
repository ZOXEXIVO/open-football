use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamAcademyRoutes;

impl TeamAcademyRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/academy",
            get(super::TeamAcademyPage::action),
        )
    }
}
