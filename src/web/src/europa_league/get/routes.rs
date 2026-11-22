use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct EuropaLeagueGetRoutes;

impl EuropaLeagueGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route("/{lang}/europa-league", get(super::EuropaLeagueGet::action))
    }
}
