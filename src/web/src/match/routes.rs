use crate::GameAppData;
use crate::r#match::chunk::{match_chunk_action, match_metadata_action};
use axum::Router;
use axum::routing::get;

pub fn match_routes() -> Router<GameAppData> {
    Router::new()
        .merge(super::get::routes::routes())
        .route("/api/match/{match_id}/metadata", get(match_metadata_action))
        .route(
            "/api/match/{match_id}/chunk/{chunk_number}",
            get(match_chunk_action),
        )
}
