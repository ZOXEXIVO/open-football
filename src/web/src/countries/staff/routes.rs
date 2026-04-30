use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/countries/{country_slug}/staff",
        get(super::country_staff_action),
    )
}
