use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct NationalCompetitionsGetRoutes;

impl NationalCompetitionsGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .route(
                "/{lang}/national-competitions",
                get(super::NationalCompetitionsGet::index),
            )
            .route(
                "/{lang}/national-competitions/{slug}",
                get(super::NationalCompetitionsGet::competition),
            )
    }
}
