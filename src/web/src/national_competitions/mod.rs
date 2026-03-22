pub mod get;

use crate::GameAppData;
use axum::Router;

pub fn national_competitions_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
}
