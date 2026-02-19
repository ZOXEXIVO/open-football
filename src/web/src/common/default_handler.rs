use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Redirect};

use rust_embed::RustEmbed;

// Include the CSS hash to force recompilation when CSS files change
// This ensures rust-embed picks up the updated styles.min.css
// Also provides CSS_VERSION for cache-busting query params
include!(concat!(env!("OUT_DIR"), "/css_hash.rs"));

#[derive(RustEmbed)]
#[folder = "assets/"]
pub struct Assets;

fn cache_control_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("woff2" | "woff" | "ttf" | "otf") => "public, max-age=31536000, immutable",
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico") => "public, max-age=86400",
        Some("css" | "js") => "public, max-age=3600",
        _ => "public, max-age=3600",
    }
}

/// Serves static files from the embedded assets, or redirects lang-less page routes
pub async fn default_handler(uri: axum::http::Uri) -> axum::response::Response {
    let path_str = uri.path().trim_start_matches('/');

    // Try serving as static asset first
    if let Some(content) = Assets::get(path_str) {
        let mime = mime_guess::from_path(path_str).first_or_octet_stream();
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime.to_string()),
                (header::CACHE_CONTROL, cache_control_for(path_str).to_string()),
            ],
            content.data,
        )
            .into_response();
    }

    // Check if path is missing a language prefix — redirect to default language
    let first_segment = path_str.split('/').next().unwrap_or("");
    let has_lang_prefix = crate::i18n::I18nManager::is_supported_language(first_segment);

    if !has_lang_prefix && !path_str.is_empty() {
        let redirect_url = format!("/{}/{}", crate::i18n::DEFAULT_LANGUAGE, path_str);
        return Redirect::permanent(&redirect_url).into_response();
    }

    (
        StatusCode::NOT_FOUND,
        [
            (header::CONTENT_TYPE, "text/plain".to_string()),
            (header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        axum::body::Bytes::from_static(b"404 Not Found"),
    )
        .into_response()
}