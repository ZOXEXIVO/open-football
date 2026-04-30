use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route(
        "/{lang}/countries/{country_slug}/free-agents",
        get(super::country_free_agents_action),
    )
}
