use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/staff/{staff_id}/personal",
        get(super::staff_personal_action),
    )
}
