use crate::date::current_date_action;
use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn current_date_routes() -> Router<GameAppData> {
    Router::new().route("/api/date", get(current_date_action))
}
