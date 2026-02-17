mod generator;
pub mod routes;

use axum::extract::{Path, Query};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use serde::Deserialize;

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

#[derive(Deserialize)]
struct FaceQueryParams {
    #[serde(default = "default_age")]
    age: u8,
}

fn default_age() -> u8 {
    25
}

async fn face_action(
    Path(path): Path<FacePathParams>,
    Query(query): Query<FaceQueryParams>,
) -> impl IntoResponse {
    let svg = generate_face_svg(path.player_id, query.age);

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (
                header::CACHE_CONTROL,
                "public, max-age=86400, immutable",
            ),
        ],
        svg,
    )
}
