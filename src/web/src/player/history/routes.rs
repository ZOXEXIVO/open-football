use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/players/{player_slug}/history",
        get(super::player_history_action),
    )
}
