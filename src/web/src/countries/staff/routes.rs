use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CountryStaffRoutes;

impl CountryStaffRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .route(
                "/{lang}/countries/{country_slug}/staff",
                get(super::CountryStaffPage::action),
            )
            .route(
                "/{lang}/countries/{country_slug}/u21/staff",
                get(super::CountryStaffPage::u21),
            )
    }
}
