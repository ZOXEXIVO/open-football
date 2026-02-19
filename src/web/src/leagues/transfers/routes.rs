use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/leagues/{league_slug}/transfers",
        get(super::league_transfers_action),
    )
}
