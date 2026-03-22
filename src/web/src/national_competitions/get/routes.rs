use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/national-competitions", get(super::national_competitions_get_action))
}
