use crate::r#match::chunk::{match_chunk_action, match_metadata_action};
use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn match_routes() -> Router<GameAppData> {
    Router::new()
        .merge(super::get::routes::routes())
        .route(
            "/api/match/{league_slug}/{match_id}/metadata",
            get(match_metadata_action),
        )
        .route(
            "/api/match/{league_slug}/{match_id}/chunk/{chunk_number}",
            get(match_chunk_action),
        )
}
