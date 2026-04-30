use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/staff/{staff_id}/personal",
        get(super::staff_personal_action),
    )
}
