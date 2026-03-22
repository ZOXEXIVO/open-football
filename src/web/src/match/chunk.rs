use crate::{ApiError, ApiResult, GameAppData};
use crate::r#match::stores::MatchStore;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct MatchChunkRequest {
    pub match_id: String,
    pub chunk_number: usize,
}

#[derive(Deserialize)]
pub struct MatchMetadataRequest {
    pub match_id: String,
}

#[derive(Serialize)]
pub struct MatchMetadataResponse {
    pub chunk_count: usize,
    pub chunk_duration_ms: u64,
    pub total_duration_ms: u64,
}

pub async fn match_chunk_action(
    State(state): State<GameAppData>,
    Path(route_params): Path<MatchChunkRequest>,
) -> ApiResult<Response> {
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    // Find league_slug from match result
    let league_slug = find_league_slug(simulator_data, &route_params.match_id);

    let chunk_data = MatchStore::get_chunk(
        &league_slug,
        &route_params.match_id,
        route_params.chunk_number,
    )
    .await
    .ok_or_else(|| {
        ApiError::NotFound(format!(
            "Chunk {} not found for match {}",
            route_params.chunk_number, route_params.match_id
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
    State(state): State<GameAppData>,
    Path(route_params): Path<MatchMetadataRequest>,
) -> ApiResult<Response> {
    let guard = state.data.read().await;
    let simulator_data = guard.as_ref().unwrap();

    let league_slug = find_league_slug(simulator_data, &route_params.match_id);

    let metadata_json = MatchStore::get_metadata(&league_slug, &route_params.match_id)
        .await
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "Chunks not available for match {}",
                route_params.match_id
            ))
        })?;

    let metadata = MatchMetadataResponse {
        chunk_count: metadata_json["chunk_count"].as_u64().unwrap_or(1) as usize,
        chunk_duration_ms: metadata_json["chunk_duration_ms"].as_u64().unwrap_or(300_000),
        total_duration_ms: metadata_json["total_duration_ms"].as_u64().unwrap_or(0),
    };

    Ok(Json(metadata).into_response())
}

/// Find the league_slug for a match by checking global store then scanning leagues
fn find_league_slug(data: &core::SimulatorData, match_id: &str) -> String {
    // Check global match store
    if let Some(mr) = data.match_store.get(match_id) {
        return mr.league_slug.clone();
    }
    // Scan leagues
    for continent in &data.continents {
        for country in &continent.countries {
            for league in &country.leagues.leagues {
                if league.matches.get(match_id).is_some() {
                    return league.slug.clone();
                }
            }
        }
    }
    "unknown".to_string()
}
