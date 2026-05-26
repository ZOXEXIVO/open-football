pub mod get;

use crate::GameAppData;
use axum::Router;

pub fn copa_libertadores_routes() -> Router<GameAppData> {
    Router::new().merge(get::routes::routes())
}
