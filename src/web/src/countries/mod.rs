pub mod free_agents;
pub mod get;
pub mod list;
pub mod schedule;
pub mod squad;
pub mod staff;

use crate::GameAppData;
use axum::Router;

pub fn country_routes() -> Router<GameAppData> {
    Router::new()
        .merge(list::routes::routes())
        .merge(get::routes::routes())
        .merge(squad::routes::routes())
        .merge(staff::routes::routes())
        .merge(schedule::routes::routes())
        .merge(free_agents::routes::routes())
}
