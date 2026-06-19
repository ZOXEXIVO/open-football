use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route(
            "/{lang}/national-competitions",
            get(super::national_competitions_get_action),
        )
        .route(
            "/{lang}/national-competitions/{slug}",
            get(super::national_competition_get_action),
        )
}
