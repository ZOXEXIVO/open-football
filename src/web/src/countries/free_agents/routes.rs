use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/countries/{country_slug}/free-agents", get(super::country_free_agents_action))
}
