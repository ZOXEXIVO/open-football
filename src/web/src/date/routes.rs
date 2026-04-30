use crate::GameAppData;
use crate::date::current_date_action;
use axum::Router;
use axum::routing::get;

pub fn current_date_routes() -> Router<GameAppData> {
    Router::new().route("/api/date", get(current_date_action))
}
