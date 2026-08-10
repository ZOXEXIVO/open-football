use crate::i18n::{DEFAULT_LANGUAGE, I18nManager};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use flate2::read::GzDecoder;

use rust_embed::RustEmbed;
use std::borrow::Cow;
use std::io::Read;
use std::sync::LazyLock;
use sysinfo::{CpuRefreshKind, RefreshKind, System};

// Include the CSS hash to force recompilation when CSS files change
// This ensures rust-embed picks up the updated styles.min.css
// Also provides CSS_VERSION for cache-busting query params
include!(concat!(env!("OUT_DIR"), "/css_hash.rs"));

// Whether `build.rs` managed to stage the WebAssembly match viewer into
// `assets/static/viewer/`, and a hash of the wasm it staged. Both come from the
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

/// Builds the response for an asset that was found, compressed or not.
fn asset_response(path: &str, data: Cow<'static, [u8]>, gzipped: bool) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(mime.as_ref())
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(cache_control_for(path)),
    );
    if gzipped {
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    (StatusCode::OK, headers, data).into_response()
}

/// Looks `path` up in the embedded assets, transparently handling the ones that
/// are only ever stored compressed.
///
/// The WebAssembly match viewer — ~30 MB inflated — is kept gzipped and served
/// under its real name, so the wasm still arrives as `application/wasm` and the
/// browser can stream-compile it. Every browser advertises gzip and gets the
/// stored bytes untouched; anything that does not (a proxy that strips the
/// header, `curl` without `--compressed`) gets it inflated here rather than a
/// 404 for a file that is demonstrably in the binary.
fn embedded_asset(path: &str, headers: &HeaderMap) -> Option<Response> {
    if let Some(content) = Assets::get(path) {
        return Some(asset_response(path, content.data, false));
    }

    let content = Assets::get(&format!("{}.gz", path))?;

    let accepts_gzip = headers
        .get(header::ACCEPT_ENCODING)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("gzip"));
    if accepts_gzip {
        return Some(asset_response(path, content.data, true));
    }

    let mut inflated = Vec::new();
    GzDecoder::new(content.data.as_ref())
        .read_to_end(&mut inflated)
        .ok()?;
    Some(asset_response(path, Cow::Owned(inflated), false))
}

fn not_found() -> Response {
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

/// Serves static files from the embedded assets, or redirects lang-less page routes
pub async fn default_handler(
    uri: axum::http::Uri,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let path_str = uri.path().trim_start_matches('/');

    if let Some(response) = embedded_asset(path_str, &headers) {
        return response;
    }

    // A missing file under `/static/` is an error, not a page that wandered off
    // without its language prefix. Redirecting it sends the browser on a chase
    // that ends at the home page, so a mistyped or unbuilt asset surfaces as a
    // stylesheet that silently does nothing or a module that fails on its MIME
    // type — several hops from the actual cause.
    if path_str.starts_with("static/") {
        return not_found();
    }

    // Check if path is missing a language prefix — redirect to default language
    let first_segment = path_str.split('/').next().unwrap_or("");
    let has_lang_prefix = I18nManager::is_supported_language(first_segment);

    if !has_lang_prefix && !path_str.is_empty() {
        let redirect_url = format!("/{}/{}", DEFAULT_LANGUAGE, path_str);
        return Redirect::permanent(&redirect_url).into_response();
    }

    not_found()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, Uri};

    /// Exactly as `index.html` asks for them. These must stay off
    /// `/static/match/`: static files reach this handler only as the router's
    /// fallback, and `/{lang}/match/{match_id}` matches that path first, with
    /// `lang` bound to `"static"`.
    const VIEWER_ASSETS: [&str; 2] = [
        "/static/viewer/match_viewer.js",
        "/static/viewer/match_viewer_bg.wasm",
    ];

    // `#[tokio::test]` expands to `::core::future`, and this workspace has its
    // own crate called `core`, which shadows the standard one — hence the
    // hand-rolled runtime.
    fn serve(path: &str, headers: HeaderMap) -> Response {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime")
            .block_on(default_handler(path.parse::<Uri>().unwrap(), headers))
    }

    fn encoding_of(response: &Response) -> Option<&str> {
        response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
    }

    /// The match viewer is the only asset stored solely compressed, so it is
    /// the only one served under a name it is not stored under.
    #[test]
    fn serves_the_gzipped_match_viewer_under_its_real_name() {
        if !MATCH_VIEWER_AVAILABLE {
            // Built on a machine with no wasm target; the page says so instead.
            return;
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br, zstd"),
        );

        for path in VIEWER_ASSETS {
            let response = serve(path, headers.clone());

            assert_eq!(response.status(), StatusCode::OK, "{path} should be served");
            assert_eq!(
                encoding_of(&response),
                Some("gzip"),
                "{path} should be handed over still compressed"
            );
        }
    }

    /// A client that cannot take gzip still gets the viewer, inflated. It used
    /// to get a 404 for a file sitting in the binary.
    #[test]
    fn serves_the_match_viewer_to_a_client_that_cannot_take_gzip() {
        if !MATCH_VIEWER_AVAILABLE {
            return;
        }

        for path in VIEWER_ASSETS {
            let response = serve(path, HeaderMap::new());

            assert_eq!(response.status(), StatusCode::OK, "{path} should be served");
            assert_eq!(
                encoding_of(&response),
                None,
                "{path} should have been inflated first"
            );
        }
    }

    /// A missing asset must fail where it failed, not bounce through the
    /// language redirect and land on the home page.
    #[test]
    fn a_missing_static_asset_is_a_plain_404() {
        assert_eq!(
            serve("/static/viewer/not_a_real_file.js", HeaderMap::new()).status(),
            StatusCode::NOT_FOUND
        );
    }
}
