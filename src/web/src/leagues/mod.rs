pub mod get;

use crate::GameAppData;
use axum::Router;

pub fn league_routes() -> Router<GameAppData> {
    get::routes::routes()
}
