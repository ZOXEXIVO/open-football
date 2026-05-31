use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/workers", get(super::workers_page_action))
}

pub fn workers_routes() -> Router<GameAppData> {
    routes()
}
