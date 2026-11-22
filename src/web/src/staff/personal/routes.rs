use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct StaffPersonalRoutes;

impl StaffPersonalRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/staff/{staff_id}/personal",
            get(super::StaffPersonalPage::action),
        )
    }
}
