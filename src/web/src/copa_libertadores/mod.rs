pub mod get;

use crate::GameAppData;
use axum::Router;

pub struct CopaLibertadoresRoutes;

impl CopaLibertadoresRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().merge(get::routes::CopaLibertadoresGetRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const copa_libertadores_routes: fn() -> Router<GameAppData> = CopaLibertadoresRoutes::router;
