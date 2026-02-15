pub mod get;
pub mod list;

use crate::GameAppData;
use axum::Router;

pub fn country_routes() -> Router<GameAppData> {
    Router::new()
        .merge(list::routes::routes())
        .merge(get::routes::routes())
}
