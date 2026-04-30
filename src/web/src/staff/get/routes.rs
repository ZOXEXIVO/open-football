use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/staff/{staff_id}", get(super::staff_get_action))
}
