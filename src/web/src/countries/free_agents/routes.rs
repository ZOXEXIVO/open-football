use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CountryFreeAgentsRoutes;

impl CountryFreeAgentsRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/countries/{country_slug}/free-agents",
            get(super::CountryFreeAgentsPage::action),
        )
    }
}
