use axum::extract::Path;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;

use rust_embed::RustEmbed;

// Include the CSS hash to force recompilation when CSS files change
// This ensures rust-embed picks up the updated styles.min.css
include!(concat!(env!("OUT_DIR"), "/css_hash.rs"));

#[derive(RustEmbed)]
#[folder = "assets/"]
pub struct Assets;

/// Serves static files from the embedded assets
pub async fn default_handler(Path(path): Path<String>) -> impl IntoResponse {
    let path_str = path.trim_start_matches('/');

    match Assets::get(path_str) {
        Some(content) => {
            let mime = mime_guess::from_path(path_str).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.to_string())],
                content.data,
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain".to_string())],
            "404 Not Found".as_bytes().into(),
        ),
    }
}

pub async fn handle_root_path() -> impl IntoResponse {
    let path_str = "static/index.html";

    match Assets::get(path_str) {
        Some(content) => {
            let mime = mime_guess::from_path(path_str).first_or_octet_stream();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, mime.to_string())],
                content.data,
            )
        }
        None => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/plain".to_string())],
            "404 Not Found".as_bytes().into(),
        ),
    }
}