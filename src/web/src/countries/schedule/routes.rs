use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CountryScheduleRoutes;

impl CountryScheduleRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .route(
                "/{lang}/countries/{country_slug}/schedule",
                get(super::CountrySchedulePage::action),
            )
            .route(
                "/{lang}/countries/{country_slug}/u21/schedule",
                get(super::CountrySchedulePage::u21),
            )
    }
}
