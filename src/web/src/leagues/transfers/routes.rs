use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct LeagueTransfersRoutes;

impl LeagueTransfersRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/leagues/{country_slug}/{league_slug}/transfers",
            get(super::LeagueTransfersPage::action),
        )
    }
}
