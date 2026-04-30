use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/match/{match_id}", get(super::match_get_action))
}
