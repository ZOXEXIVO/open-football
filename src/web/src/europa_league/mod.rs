pub mod get;

use crate::GameAppData;
use axum::Router;

pub struct EuropaLeagueRoutes;

impl EuropaLeagueRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().merge(get::routes::EuropaLeagueGetRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const europa_league_routes: fn() -> Router<GameAppData> = EuropaLeagueRoutes::router;
