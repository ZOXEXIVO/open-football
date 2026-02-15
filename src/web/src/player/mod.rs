pub mod get;

pub use get::PlayerStatusDto;

use crate::GameAppData;
use axum::Router;

pub fn player_routes() -> Router<GameAppData> {
    get::routes::routes()
}
