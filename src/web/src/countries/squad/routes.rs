use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route(
            "/{lang}/countries/{country_slug}",
            get(super::country_squad_action),
        )
        .route(
            "/{lang}/countries/{country_slug}/u21",
            get(super::country_u21_squad_action),
        )
}
