use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CountryListRoutes;

impl CountryListRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .route("/{lang}", get(super::CountryListPage::action))
            .route("/{lang}/countries", get(super::CountryListPage::action))
    }
}
