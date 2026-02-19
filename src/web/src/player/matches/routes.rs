use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/players/{player_id}/matches",
        get(super::player_matches_action),
    )
}
