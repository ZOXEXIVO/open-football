use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/countries/{country_slug}",
        get(super::country_squad_action),
    )
}
