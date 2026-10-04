pub mod address;
pub mod awards;
pub mod get;
pub mod newspaper;
pub mod transfers;

use crate::GameAppData;
use crate::leagues::address::league_moved_action;
use axum::Router;
use axum::routing::get;

pub fn league_routes() -> Router<GameAppData> {
    Router::new()
        .merge(get::routes::routes())
        .merge(newspaper::routes::routes())
        .merge(transfers::routes::routes())
        .merge(awards::routes::routes())
        // Pre-country URLs, kept as permanent redirects so old links still
        // resolve.
        .route("/{lang}/leagues/{league_slug}", get(league_moved_action))
        .route(
            "/{lang}/leagues/{league_slug}/newspaper",
            get(league_moved_action),
        )
        .route(
            "/{lang}/leagues/{league_slug}/transfers",
            get(league_moved_action),
        )
        .route(
            "/{lang}/leagues/{league_slug}/awards",
            get(league_moved_action),
        )
}
