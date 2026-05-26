use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/copa-libertadores",
        get(super::copa_libertadores_get_action),
    )
}
