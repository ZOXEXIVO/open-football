pub mod get;
pub mod personal;

use crate::GameAppData;
use axum::Router;

pub fn staff_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(personal::routes::routes())
}
