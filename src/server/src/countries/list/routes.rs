use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route("/", get(super::country_list_action))
        .route("/countries", get(super::country_list_action))
}
