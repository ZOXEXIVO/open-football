use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/teams/{team_slug}/finances",
        get(super::team_finances_get_action),
    )
}
