use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route("/{lang}/search", get(super::search_page_action))
        .route("/api/search", get(super::search_api_action))
}
