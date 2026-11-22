use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CopaLibertadoresGetRoutes;

impl CopaLibertadoresGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/copa-libertadores",
            get(super::CopaLibertadoresGet::action),
        )
    }
}
