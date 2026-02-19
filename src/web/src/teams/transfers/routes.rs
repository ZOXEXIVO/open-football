use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/teams/{team_slug}/transfers",
        get(super::team_transfers_action),
    )
}
