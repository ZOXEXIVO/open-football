use crate::GameAppData;
use crate::watchlist::{WatchlistPageTemplate, WatchlistRowsTemplate};
use axum::Router;
use axum::routing::{get, post};

pub struct WatchlistRoutes;

impl WatchlistRoutes {
    pub fn routes() -> Router<GameAppData> {
        Router::new()
            .route("/{lang}/watchlist", get(WatchlistPageTemplate::action))
            .route(
                "/{lang}/watchlist/rows",
                post(WatchlistRowsTemplate::action),
            )
    }
}
