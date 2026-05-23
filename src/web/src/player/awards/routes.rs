use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/players/{player_slug}/awards",
        get(super::player_awards_action),
    )
}
