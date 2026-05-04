use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/leagues/{league_slug}/awards",
        get(super::league_awards_action),
    )
}
