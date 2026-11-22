use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CountryGetRoutes;

impl CountryGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/countries/{country_slug}/leagues",
            get(super::CountryGetPage::action),
        )
    }
}
