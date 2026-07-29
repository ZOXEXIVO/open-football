use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/leagues/{league_slug}/newspaper",
        get(super::league_newspaper_action),
    )
}
