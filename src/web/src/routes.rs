use crate::countries::country_routes;
use crate::date::current_date_routes;
use crate::face::face_routes;
use crate::game::game_routes;
use crate::i18n::detect_language;
use crate::leagues::league_routes;
use crate::player::player_routes;
use crate::r#match::routes::match_routes;
use crate::staff::staff_routes;
use crate::teams::team_routes;
use crate::GameAppData;
use axum::http::header::ACCEPT_LANGUAGE;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect};
use axum::routing::get;
use axum::Router;
use crate::common::default_handler::default_handler;

async fn root_redirect(headers: HeaderMap) -> impl IntoResponse {
    let accept_language = headers
        .get(ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("en");
    let lang = detect_language(accept_language);
    Redirect::temporary(&format!("/{}", lang))
}

pub struct ServerRoutes;

impl ServerRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::<GameAppData>::new()
            .route("/", get(root_redirect))
            .merge(country_routes())
            .merge(game_routes())
            .merge(league_routes())
            .merge(team_routes())
            .merge(player_routes())
            .merge(staff_routes())
            .merge(match_routes())
            .merge(current_date_routes())
            .merge(face_routes())
            .fallback(default_handler)
    }
}
