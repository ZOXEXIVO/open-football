pub mod get;

use crate::GameAppData;
use axum::Router;

pub struct ChampionsLeagueRoutes;

impl ChampionsLeagueRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().merge(get::routes::ChampionsLeagueGetRoutes::router())
    }
}

#[allow(non_upper_case_globals)]
pub const champions_league_routes: fn() -> Router<GameAppData> = ChampionsLeagueRoutes::router;
