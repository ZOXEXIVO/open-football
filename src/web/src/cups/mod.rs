pub mod get;
pub mod history;

use crate::GameAppData;
use axum::Router;

pub struct CupRoutes;

impl CupRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .merge(get::routes::CupGetRoutes::router())
            .merge(history::routes::CupHistoryRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const cup_routes: fn() -> Router<GameAppData> = CupRoutes::router;
