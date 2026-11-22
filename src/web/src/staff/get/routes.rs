use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct StaffGetRoutes;

impl StaffGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route("/{lang}/staff/{staff_id}", get(super::StaffGetPage::action))
    }
}
