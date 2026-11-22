pub mod address;
pub mod awards;
pub mod get;
pub mod newspaper;
pub mod transfers;

use crate::GameAppData;
use crate::leagues::address::LeagueAddress;
use axum::Router;
use axum::routing::get;

pub struct LeagueRoutes;

impl LeagueRoutes {
    pub fn router() -> Router<GameAppData> {
        Router::new()
            .merge(get::routes::LeagueGetRoutes::router())
            .merge(newspaper::routes::LeagueNewspaperRoutes::router())
            .merge(transfers::routes::LeagueTransfersRoutes::router())
            .merge(awards::routes::LeagueAwardsRoutes::router())
            // Pre-country URLs, kept as permanent redirects so old links still
            // resolve.
            .route(
                "/{lang}/leagues/{league_slug}",
                get(LeagueAddress::moved_action),
            )
            .route(
                "/{lang}/leagues/{league_slug}/newspaper",
                get(LeagueAddress::moved_action),
            )
            .route(
                "/{lang}/leagues/{league_slug}/transfers",
                get(LeagueAddress::moved_action),
            )
            .route(
                "/{lang}/leagues/{league_slug}/awards",
                get(LeagueAddress::moved_action),
            )
    }
}

#[allow(non_upper_case_globals)]
pub const league_routes: fn() -> Router<GameAppData> = LeagueRoutes::router;
