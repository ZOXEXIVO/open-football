use crate::GameAppData;
use crate::r#match::stores::MatchStore;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct MatchChunkRequest {
    pub league_slug: String,
    pub match_id: String,
    pub chunk_number: usize,
}

#[derive(Deserialize)]
pub struct MatchMetadataRequest {
    pub league_slug: String,
    pub match_id: String,
}

#[derive(Serialize)]
pub struct MatchMetadataResponse {
    pub chunk_count: usize,
    pub chunk_duration_ms: u64,
    pub total_duration_ms: u64,
}

pub async fn match_chunk_action(
    State(_): State<GameAppData>,
    Path(route_params): Path<MatchChunkRequest>,
) -> Response {
    let chunk_data = MatchStore::get_chunk(
        &route_params.league_slug,
        &route_params.match_id,
        route_params.chunk_number,
    )
    .await;

    match chunk_data {
        Some(data) => {
            let mut response = (StatusCode::OK, data).into_response();

            response
                .headers_mut()
                .append("Content-Type", "application/gzip".parse().unwrap());
            response
                .headers_mut()
                .append("Content-Encoding", "gzip".parse().unwrap());

            response
        }
        None => (StatusCode::NOT_FOUND, "Chunk not found").into_response(),
    }
}

pub async fn match_metadata_action(
    State(_): State<GameAppData>,
    Path(route_params): Path<MatchMetadataRequest>,
) -> Response {
    let metadata_json = MatchStore::get_metadata(&route_params.league_slug, &route_params.match_id).await;

    match metadata_json {
        Some(meta) => {
            let metadata = MatchMetadataResponse {
                chunk_count: meta["chunk_count"].as_u64().unwrap_or(1) as usize,
                chunk_duration_ms: meta["chunk_duration_ms"].as_u64().unwrap_or(300_000),
                total_duration_ms: meta["total_duration_ms"].as_u64().unwrap_or(0),
            };
            Json(metadata).into_response()
        }
        None => {
            (StatusCode::NOT_FOUND, "Chunks not available for this match").into_response()
        }
    }
}
