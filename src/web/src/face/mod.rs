mod generator;
pub mod routes;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use core::utils::DateUtils;
use generator::generate_face_svg;

use crate::GameAppData;
use axum::Router;

pub fn face_routes() -> Router<GameAppData> {
    routes::routes()
}

#[derive(Deserialize)]
struct FacePathParams {
    player_id: u32,
}

async fn face_action(
    State(state): State<GameAppData>,
    Path(path): Path<FacePathParams>,
) -> Response {
    let guard = state.data.read().await;
    let Some(simulator_data) = guard.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    let Some(player) = simulator_data.player(path.player_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let age = DateUtils::age(player.birth_date, simulator_data.date.date());

    let country_code = simulator_data
        .country(player.country_id)
        .map(|c| c.code.clone())
        .or_else(|| {
            simulator_data
                .country_info
                .get(&player.country_id)
                .map(|i| i.code.clone())
        })
        .unwrap_or_default();

    let skin_dist = generator::skin_distribution_for_country(&country_code);
    let svg = generate_face_svg(path.player_id, age, skin_dist);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        svg,
    )
        .into_response()
}
