use axum::Router;
use axum::routing::get;
use crate::common::default_handler::default_handler;
use crate::GameAppData;

pub fn current_common_routes() -> Router<GameAppData> {
    Router::new().route("/{*path}", get(default_handler))
}
