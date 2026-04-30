use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/teams/{team_slug}", get(super::team_get_action))
}
