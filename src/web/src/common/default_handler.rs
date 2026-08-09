use crate::i18n::{DEFAULT_LANGUAGE, I18nManager};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect};

use rust_embed::RustEmbed;
use std::sync::LazyLock;
use sysinfo::{CpuRefreshKind, RefreshKind, System};

// Include the CSS hash to force recompilation when CSS files change
// This ensures rust-embed picks up the updated styles.min.css
// Also provides CSS_VERSION for cache-busting query params
include!(concat!(env!("OUT_DIR"), "/css_hash.rs"));

// Whether `build.rs` managed to stage the WebAssembly match viewer into
// `assets/static/match/`, and a hash of the wasm it staged. Both come from the
// same build-script pass that compiles `src/match`.
include!(concat!(env!("OUT_DIR"), "/match_viewer.rs"));

/// Machine hostname, resolved once at startup.
pub static COMPUTER_NAME: LazyLock<String> = LazyLock::new(|| {
    hostname::get()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string())
});

/// Logical CPU thread count, resolved once at startup.
pub static CPU_CORES: LazyLock<usize> = LazyLock::new(|| {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
});

/// CPU brand string (e.g. "AMD Ryzen 9 7950X 16-Core Processor"), resolved once at startup.
pub static CPU_BRAND: LazyLock<String> = LazyLock::new(|| {
    let sys =
        System::new_with_specifics(RefreshKind::nothing().with_cpu(CpuRefreshKind::nothing()));
    sys.cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Unknown CPU".to_string())
});

#[derive(RustEmbed)]
#[folder = "assets/"]
pub struct Assets;

fn cache_control_for(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("woff2" | "woff" | "ttf" | "otf") => "public, max-age=31536000, immutable",
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico") => "public, max-age=86400",
        // The viewer module and its wasm are requested with a build hash in the
        // query string, so they can be cached hard.
        Some("wasm") => "public, max-age=31536000, immutable",
        Some("css" | "js") => "public, max-age=3600",
        _ => "public, max-age=3600",
    }
}

/// Serves static files from the embedded assets, or redirects lang-less page routes
pub async fn default_handler(
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let path_str = uri.path().trim_start_matches('/');

    // Try serving as static asset first
    if let Some(content) = Assets::get(path_str) {
        let mime = mime_guess::from_path(path_str).first_or_octet_stream();
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime.to_string()),
                (
                    header::CACHE_CONTROL,
                    cache_control_for(path_str).to_string(),
                ),
            ],
            content.data,
        )
            .into_response();
    }

    // Assets that are only ever stored compressed — the WebAssembly match
    // viewer and its module, ~30 MB inflated — live next door under a `.gz`
    // suffix and are handed straight to the browser to inflate. They keep their
    // real name in the URL so the wasm still arrives as `application/wasm`,
    // which is what lets the browser stream-compile it.
    let accepts_gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("gzip"));
    if accepts_gzip {
        if let Some(content) = Assets::get(&format!("{}.gz", path_str)) {
            let mime = mime_guess::from_path(path_str).first_or_octet_stream();
            return (
                StatusCode::OK,
                [
                    (header::CONTENT_TYPE, mime.to_string()),
                    (header::CONTENT_ENCODING, "gzip".to_string()),
                    (
                        header::CACHE_CONTROL,
                        cache_control_for(path_str).to_string(),
                    ),
                ],
                content.data,
            )
                .into_response();
        }
    }

    // Check if path is missing a language prefix — redirect to default language
    let first_segment = path_str.split('/').next().unwrap_or("");
    let has_lang_prefix = I18nManager::is_supported_language(first_segment);

    if !has_lang_prefix && !path_str.is_empty() {
        let redirect_url = format!("/{}/{}", DEFAULT_LANGUAGE, path_str);
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
