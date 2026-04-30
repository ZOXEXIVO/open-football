use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/leagues/{league_slug}/transfers",
        get(super::league_transfers_action),
    )
}
