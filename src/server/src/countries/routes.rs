use crate::countries::{country_get_action, country_list_action};
use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn country_routes() -> Router<GameAppData> {
    Router::new()
        .route("/api/countries", get(country_list_action))
        .route("/api/countries/{country_slug}", get(country_get_action))
}
