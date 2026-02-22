pub mod get;
pub mod list;
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
}
