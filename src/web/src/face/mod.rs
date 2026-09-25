mod beard;
mod canvas;
mod color;
mod features;
mod generator;
mod geometry;
mod hair;
mod identity;
mod noise;
pub mod routes;
mod shading;
pub mod skin;
mod tones;

/// Cache-busting version for face URLs. Responses are served `immutable`,
/// so bump this whenever generator output changes — every template injects
/// it via `{{ crate::face::FACE_VERSION }}`.
pub const FACE_VERSION: u32 = 15;

/// Where the real head shots live: the picture library every `<img>` on the
/// site already points at, and the first thing the match viewer tries for a
/// player who is a real footballer rather than a regen.
///
/// Held here rather than spelled out at each use so the match page and the
/// portrait route agree about what a player's picture is, and so moving the
/// library is one edit.
pub const PHOTO_LIBRARY: &str = "https://open-football.org/player";

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use core::utils::DateUtils;
use generator::{FaceFrame, Portrait, Sitter};
use skin::CountrySkin;

use crate::GameAppData;
use axum::Router;

pub fn face_routes() -> Router<GameAppData> {
    routes::routes()
}

#[derive(Deserialize)]
struct FacePathParams {
    player_id: u32,
}

/// The profile-page portrait, on a studio card.
async fn portrait_action(state: State<GameAppData>, path: Path<FacePathParams>) -> Response {
    face_response(state, path, FaceFrame::Portrait).await
}

/// The head alone on transparent ground, for the match viewer — see
/// [`FaceFrame::Cutout`].
async fn cutout_action(state: State<GameAppData>, path: Path<FacePathParams>) -> Response {
    face_response(state, path, FaceFrame::Cutout).await
}

async fn face_response(
    State(state): State<GameAppData>,
    Path(path): Path<FacePathParams>,
    frame: FaceFrame,
) -> Response {
    let guard = state.data.read().await;
    let Some(simulator_data) = guard.as_ref() else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };

    let Some(player) = simulator_data.player(path.player_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let age = DateUtils::age(player.birth_date, simulator_data.date.date());

    let skin = CountrySkin::for_country(simulator_data, player.country_id);

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

    drop(guard);

    let sitter = Sitter {
        player_id: path.player_id,
        age,
        skin,
        heft,
        aggression,
    };
    let Some(bytes) = Portrait::commission(sitter, frame).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, frame.mime()),
            (header::CACHE_CONTROL, "public, max-age=86400, immutable"),
        ],
        bytes,
    )
        .into_response()
}
