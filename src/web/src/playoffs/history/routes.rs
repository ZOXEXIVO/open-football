use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/playoffs/{playoff_slug}/history",
        get(super::playoff_history_action),
    )
}
