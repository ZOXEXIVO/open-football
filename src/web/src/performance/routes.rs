use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new().route("/{lang}/performance", get(super::performance_page_action))
}
