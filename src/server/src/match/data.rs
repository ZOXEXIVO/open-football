use crate::GameAppData;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use crate::r#match::stores::MatchStore;

#[derive(Deserialize)]
pub struct MatchDataRequest {
    pub league_slug: String,
    pub match_id: String,
}

pub async fn match_data_action(
    State(_): State<GameAppData>,
    Path(route_params): Path<MatchDataRequest>,
) -> Response {
    let match_data = MatchStore::get(&route_params.league_slug, &route_params.match_id).await;

    let mut response = (StatusCode::OK, match_data).into_response();

    response
        .headers_mut()
        .append("Content-Type", "application/gzip".parse().unwrap());
    response
        .headers_mut()
        .append("Content-Encoding", "gzip".parse().unwrap());

    return response;
}