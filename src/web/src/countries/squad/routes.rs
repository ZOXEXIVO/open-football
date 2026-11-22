use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CountrySquadRoutes;

impl CountrySquadRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .route(
                "/{lang}/countries/{country_slug}",
                get(super::CountrySquadPage::action),
            )
            .route(
                "/{lang}/countries/{country_slug}/u21",
                get(super::CountrySquadPage::u21),
            )
    }
}
