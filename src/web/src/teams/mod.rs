pub mod get;
pub mod schedule;
pub mod staff;
pub mod stats;
pub mod tactics;
pub mod transfers;

use crate::GameAppData;
use axum::Router;

pub fn team_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(staff::routes::routes())
        .merge(tactics::routes::routes())
        .merge(schedule::routes::routes())
        .merge(stats::routes::routes())
        .merge(transfers::routes::routes())
}
