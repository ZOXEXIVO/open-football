use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct ChampionsLeagueGetRoutes;

impl ChampionsLeagueGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/champions-league",
            get(super::ChampionsLeagueGet::action),
        )
    }
}
