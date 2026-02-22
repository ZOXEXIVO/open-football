use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/countries/{country_slug}/schedule",
        get(super::country_schedule_action),
    )
}
