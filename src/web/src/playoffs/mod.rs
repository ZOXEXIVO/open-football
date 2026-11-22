pub mod get;
pub mod history;

use crate::GameAppData;
use axum::Router;

pub struct PlayoffRoutes;

impl PlayoffRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .merge(get::routes::PlayoffGetRoutes::router())
            .merge(history::routes::PlayoffHistoryRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const playoff_routes: fn() -> Router<GameAppData> = PlayoffRoutes::router;
