use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/leagues/{country_slug}/{league_slug}/awards",
        get(super::league_awards_action),
    )
}
