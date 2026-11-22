pub mod get;
pub mod personal;

use crate::GameAppData;
use axum::Router;

pub struct StaffRoutes;

impl StaffRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .merge(get::routes::StaffGetRoutes::router())
            .merge(personal::routes::StaffPersonalRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const staff_routes: fn() -> Router<GameAppData> = StaffRoutes::router;
