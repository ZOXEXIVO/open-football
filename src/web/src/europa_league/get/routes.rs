use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/europa-league", get(super::europa_league_get_action))
}
