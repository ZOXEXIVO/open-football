use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct MatchGetRoutes;

impl MatchGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route("/{lang}/match/{match_id}", get(super::MatchGetPage::action))
    }
}
