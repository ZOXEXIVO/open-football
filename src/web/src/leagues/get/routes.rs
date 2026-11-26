use super::highlights::LeagueHighlights;
use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct LeagueGetRoutes;

impl LeagueGetRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .route(
                "/{lang}/leagues/{country_slug}/{league_slug}",
                get(super::LeagueGetPage::action),
            )
            .route(
                "/{lang}/leagues/{country_slug}/{league_slug}/match-highlights",
                get(LeagueHighlights::action),
            )
    }
}
