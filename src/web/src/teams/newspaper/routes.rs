use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct TeamNewspaperRoutes;

impl TeamNewspaperRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/teams/{team_slug}/newspaper",
            get(super::NewspaperPage::action),
        )
    }
}
