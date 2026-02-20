use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/staff/{staff_id}",
        get(super::staff_get_action),
    )
}
