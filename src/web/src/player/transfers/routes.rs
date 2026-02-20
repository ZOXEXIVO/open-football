use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/players/{player_id}/transfers",
        get(super::player_transfers_action),
    )
}
