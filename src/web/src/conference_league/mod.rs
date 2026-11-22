pub mod get;

use crate::GameAppData;
use axum::Router;

pub struct ConferenceLeagueRoutes;

impl ConferenceLeagueRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().merge(get::routes::ConferenceLeagueGetRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const conference_league_routes: fn() -> Router<GameAppData> = ConferenceLeagueRoutes::router;
