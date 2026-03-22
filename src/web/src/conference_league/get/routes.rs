use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/conference-league", get(super::conference_league_get_action))
}
