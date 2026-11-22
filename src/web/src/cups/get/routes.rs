use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct CupGetRoutes;

impl CupGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route("/{lang}/cups/{cup_slug}", get(super::CupGetPage::action))
    }
}
