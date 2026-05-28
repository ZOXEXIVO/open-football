pub mod get;
pub mod history;

use crate::GameAppData;
use axum::Router;

pub fn cup_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(history::routes::routes())
}
