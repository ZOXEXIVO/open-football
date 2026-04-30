use crate::GameAppData;
use crate::ai::ai_routes;
use crate::champions_league::champions_league_routes;
use crate::common::default_handler::default_handler;
use crate::conference_league::conference_league_routes;
use crate::countries::country_routes;
use crate::date::current_date_routes;
use crate::europa_league::europa_league_routes;
use crate::face::face_routes;
use crate::game::game_routes;
use crate::i18n::{SUPPORTED_LANG_CODES, detect_language};
use crate::leagues::league_routes;
use crate::r#match::routes::match_routes;
use crate::national_competitions::national_competitions_routes;
use crate::player::player_routes;
use crate::search::search_routes;
use crate::staff::staff_routes;
use crate::teams::team_routes;
use crate::watchlist::watchlist_routes;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::HeaderMap;
use axum::http::header::ACCEPT_LANGUAGE;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;

async fn root_redirect(headers: HeaderMap) -> impl IntoResponse {
    let accept_language = headers
        .get(ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("en");
    let lang = detect_language(accept_language);
    Redirect::temporary(&format!("/{}", lang))
}

async fn sitemap_xml(State(state): State<GameAppData>) -> impl IntoResponse {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
        <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );

    // Language root pages — monthly
    for lang in SUPPORTED_LANG_CODES {
        xml.push_str(&format!(
            "  <url>\n    <loc>https://open-football.org/{}</loc>\n    <lastmod>{}</lastmod>\n    <changefreq>monthly</changefreq>\n  </url>\n",
            lang, date
        ));
    }

    // All club team pages — daily
    let guard = state.data.read().await;
    if let Some(ref sim) = *guard {
        for continent in &sim.continents {
            for country in &continent.countries {
                for club in &country.clubs {
                    for team in &club.teams.teams {
                        if team.team_type != core::TeamType::Main {
                            continue;
                        }
                        for lang in SUPPORTED_LANG_CODES {
                            xml.push_str(&format!(
                                "  <url>\n    <loc>https://open-football.org/{}/teams/{}</loc>\n    <lastmod>{}</lastmod>\n    <changefreq>daily</changefreq>\n  </url>\n",
                                lang, team.slug, date
                            ));
                        }
                    }
                }
            }
        }
    }

    xml.push_str("</urlset>\n");

    ([(axum::http::header::CONTENT_TYPE, "application/xml")], xml)
}

/// Middleware that turns user-facing errors into redirects to the home page.
///
/// Pre-handler: rejects unsupported language prefixes (e.g. `/saas/countries/...`).
/// Post-handler: catches any 4xx response (not-found entity, invalid path param).
async fn redirect_on_error(request: Request, next: Next) -> Response {
    let path = request.uri().path();

    // Never redirect API endpoints or static assets
    if path.starts_with("/api/") || path.starts_with("/static/") {
        return next.run(request).await;
    }

    // Check language prefix before running the handler
    let first_segment = path.trim_start_matches('/').split('/').next().unwrap_or("");

    // Only validate paths that look like /{lang}/... (skip `/`, `/sitemap.xml`, assets)
    if !first_segment.is_empty()
        && path.matches('/').count() > 1
        && !SUPPORTED_LANG_CODES.contains(&first_segment)
    {
        return Redirect::temporary("/").into_response();
    }

    let response = next.run(request).await;

    if response.status().is_client_error() {
        Redirect::temporary("/").into_response()
    } else {
        response
    }
}

pub struct ServerRoutes;

impl ServerRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::<GameAppData>::new()
            .route("/", get(root_redirect))
            .route("/sitemap.xml", get(sitemap_xml))
            .merge(champions_league_routes())
            .merge(europa_league_routes())
            .merge(conference_league_routes())
            .merge(national_competitions_routes())
            .merge(country_routes())
            .merge(game_routes())
            .merge(league_routes())
            .merge(team_routes())
            .merge(player_routes())
            .merge(staff_routes())
            .merge(match_routes())
            .merge(current_date_routes())
            .merge(face_routes())
            .merge(watchlist_routes())
            .merge(search_routes())
            .merge(ai_routes())
            .fallback(default_handler)
            .layer(axum::middleware::from_fn(redirect_on_error))
    }
}
