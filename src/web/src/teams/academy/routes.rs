use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/teams/{team_slug}/academy",
        get(super::team_academy_action),
    )
}
