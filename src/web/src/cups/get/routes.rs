use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/cups/{cup_slug}", get(super::cup_get_action))
}
