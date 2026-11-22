use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct ConferenceLeagueGetRoutes;

impl ConferenceLeagueGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/conference-league",
            get(super::ConferenceLeagueGet::action),
        )
    }
}
