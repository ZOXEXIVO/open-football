use crate::{ApiError, ApiResult, GameAppData};
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
) -> ApiResult<Response> {
    let chunk_data = MatchStore::get_chunk(
        &route_params.league_slug,
        &route_params.match_id,
        route_params.chunk_number,
    )
    .await
    .ok_or_else(|| {
        ApiError::NotFound(format!(
            "Chunk {} not found for match {}/{}",
            route_params.chunk_number, route_params.league_slug, route_params.match_id
        ))
    })?;

    let mut response = (StatusCode::OK, chunk_data).into_response();

    response
        .headers_mut()
        .append(
            "Content-Type",
            "application/gzip"
                .parse()
                .map_err(|e| ApiError::InternalError(format!("Header parse error: {:?}", e)))?,
        );
    response
        .headers_mut()
        .append(
            "Content-Encoding",
            "gzip"
                .parse()
                .map_err(|e| ApiError::InternalError(format!("Header parse error: {:?}", e)))?,
        );

    Ok(response)
}

pub async fn match_metadata_action(
    State(_): State<GameAppData>,
    Path(route_params): Path<MatchMetadataRequest>,
) -> ApiResult<Response> {
    let metadata_json = MatchStore::get_metadata(&route_params.league_slug, &route_params.match_id)
        .await
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Chunks not available for match {}/{}",
                route_params.league_slug, route_params.match_id
            ))
        })?;

    let metadata = MatchMetadataResponse {
        chunk_count: metadata_json["chunk_count"].as_u64().unwrap_or(1) as usize,
        chunk_duration_ms: metadata_json["chunk_duration_ms"].as_u64().unwrap_or(300_000),
        total_duration_ms: metadata_json["total_duration_ms"].as_u64().unwrap_or(0),
    };

    Ok(Json(metadata).into_response())
}
