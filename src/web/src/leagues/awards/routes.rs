use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct LeagueAwardsRoutes;

impl LeagueAwardsRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/leagues/{country_slug}/{league_slug}/awards",
            get(super::LeagueAwardsPage::action),
        )
    }
}
