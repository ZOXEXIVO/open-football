use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/leagues/{country_slug}/{league_slug}",
        get(super::league_get_action),
    )
}
