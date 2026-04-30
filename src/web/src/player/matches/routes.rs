use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/players/{player_slug}/matches",
        get(super::player_matches_action),
    )
}
