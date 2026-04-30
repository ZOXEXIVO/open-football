use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/countries/{country_slug}/schedule",
        get(super::country_schedule_action),
    )
}
