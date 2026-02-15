use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/teams/{team_slug}/schedule",
        get(super::team_schedule_get_action),
    )
}
