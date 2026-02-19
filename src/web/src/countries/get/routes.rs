use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/countries/{country_slug}", get(super::country_get_action))
}
