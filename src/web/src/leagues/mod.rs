pub mod get;
pub mod transfers;

use crate::GameAppData;
use axum::Router;

pub fn league_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(transfers::routes::routes())
}
