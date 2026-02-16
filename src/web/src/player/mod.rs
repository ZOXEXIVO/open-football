pub mod get;
pub mod history;
pub mod matches;

pub use get::PlayerStatusDto;

use crate::GameAppData;
use axum::Router;

pub fn player_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(matches::routes::routes())
        .merge(history::routes::routes())
}
