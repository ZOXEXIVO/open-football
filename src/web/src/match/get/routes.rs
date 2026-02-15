use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/match/{league_slug}/{match_id}", get(super::match_get_action))
}
