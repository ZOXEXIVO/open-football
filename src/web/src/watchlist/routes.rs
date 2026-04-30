use crate::GameAppData;
use axum::Router;
use axum::routing::{get, post};

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route("/{lang}/watchlist", get(super::watchlist_page_action))
        .route(
            "/api/watchlist/add/{player_id}",
            post(super::watchlist_add_action),
        )
        .route(
            "/api/watchlist/remove/{player_id}",
            post(super::watchlist_remove_action),
        )
}
