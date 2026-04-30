pub mod get;

use crate::GameAppData;
use axum::Router;

pub fn champions_league_routes() -> Router<GameAppData> {
    Router::new().merge(get::routes::routes())
}
