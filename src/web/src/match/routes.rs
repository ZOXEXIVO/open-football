use crate::GameAppData;
use crate::r#match::chunk::{MatchChunk, MatchMetadata};
use axum::Router;
use axum::routing::get;

pub struct MatchRoutes;

impl MatchRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .merge(super::get::routes::MatchGetRoutes::router())
            .route("/api/match/{match_id}/metadata", get(MatchMetadata::action))
            .route(
                "/api/match/{match_id}/chunk/{chunk_number}",
                get(MatchChunk::action),
            )
    }
}

#[allow(non_upper_case_globals)]
pub const match_routes: fn() -> Router<GameAppData> = MatchRoutes::router;
