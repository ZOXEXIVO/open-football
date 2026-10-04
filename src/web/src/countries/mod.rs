pub mod free_agents;
pub mod get;
pub mod list;
pub mod schedule;
pub mod squad;
pub mod staff;

use crate::GameAppData;
use axum::Router;
use axum::extract::{Path, Request};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use core::shared::fullname::slug_from_display;
use std::collections::HashMap;

pub fn country_routes() -> Router<GameAppData> {
    Router::new()
        .merge(list::routes::routes())
        .merge(get::routes::routes())
        .merge(squad::routes::routes())
        .merge(staff::routes::routes())
        .merge(schedule::routes::routes())
        .merge(free_agents::routes::routes())
        .route_layer(middleware::from_fn(CountrySlug::canonical))
}

/// Country slugs are folded at load ("south africa" → "south-africa"), so a
/// `{country_slug}` the fold would still change comes from a pre-fold URL.
struct CountrySlug;

impl CountrySlug {
    /// Moves such a URL permanently to the folded slug, tab and query intact.
    /// Built by hand because `Redirect::permanent` is a 308, and 301 is the
    /// status browsers and crawlers cache as a permanent move.
    async fn canonical(
        Path(params): Path<HashMap<String, String>>,
        request: Request,
        next: Next,
    ) -> Response {
        let Some(slug) = params.get("country_slug") else {
            return next.run(request).await;
        };
        let folded = slug_from_display(slug);
        if folded == *slug {
            return next.run(request).await;
        }

        Self::moved(request.uri().path(), request.uri().query(), &folded)
    }

    /// Every country route is `/{lang}/countries/{country_slug}…`, so the
    /// slug is the fourth `/`-separated piece of the path.
    fn moved(path: &str, query: Option<&str>, folded: &str) -> Response {
        let mut segments: Vec<&str> = path.split('/').collect();
        segments[3] = folded;
        let mut url = segments.join("/");
        if let Some(query) = query {
            url.push('?');
            url.push_str(query);
        }
        (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, url)]).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pre_fold_country_url_moves_to_the_folded_slug_on_the_same_tab() {
        let response = CountrySlug::moved(
            "/en/countries/south%20africa/u21/schedule",
            Some("season=2025"),
            "south-africa",
        );

        assert_eq!(response.status(), StatusCode::MOVED_PERMANENTLY);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("a redirect must carry a Location"),
            "/en/countries/south-africa/u21/schedule?season=2025"
        );
    }
}
