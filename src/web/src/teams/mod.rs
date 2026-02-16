pub mod get;
pub mod history;
pub mod schedule;
pub mod stats;
pub mod tactics;
pub mod transfers;

use crate::GameAppData;
use axum::Router;

pub fn team_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(tactics::routes::routes())
        .merge(schedule::routes::routes())
        .merge(stats::routes::routes())
        .merge(transfers::routes::routes())
        .merge(history::routes::routes())
}
