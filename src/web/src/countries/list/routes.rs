use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route("/{lang}", get(super::country_list_action))
        .route("/{lang}/countries", get(super::country_list_action))
}
