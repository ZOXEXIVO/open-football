use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/teams/{team_slug}/tactics",
        get(super::team_tactics_get_action),
    )
}
