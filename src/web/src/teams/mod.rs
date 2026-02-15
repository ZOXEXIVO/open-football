pub mod get;
pub mod schedule;

use crate::GameAppData;
use axum::Router;

pub fn team_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(schedule::routes::routes())
}
