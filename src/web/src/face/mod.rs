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

    // Weight-for-height drives facial fullness; fall back to an average
    // build when the record carries no plausible body data
    let height_cm = if player.player_attributes.height >= 150 {
        player.player_attributes.height as f32
    } else {
        180.0
    };
    let weight_kg = if player.player_attributes.weight >= 45 {
        player.player_attributes.weight as f32
    } else {
        75.0
    };
    let athletic_kg = 23.0 * (height_cm / 100.0) * (height_cm / 100.0);
    let heft = (weight_kg - athletic_kg) / 6.0;

    // Expression: short fuse (low temperament) + dirty tackling read as a
    // harder face; both attributes are on the 0..20 scale
    let aggression =
        (((20.0 - player.attributes.temperament) * 0.6 + player.attributes.dirtiness * 0.4) / 20.0)
            .clamp(0.0, 1.0);

    let svg = generate_face_svg(path.player_id, age, skin_dist, heft, aggression);

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
