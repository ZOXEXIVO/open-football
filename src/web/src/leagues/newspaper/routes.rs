use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct LeagueNewspaperRoutes;

impl LeagueNewspaperRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/leagues/{country_slug}/{league_slug}/newspaper",
            get(super::LeagueNewspaperPage::action),
        )
    }
}
