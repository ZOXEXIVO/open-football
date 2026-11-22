use crate::GameAppData;
use crate::watchlist::{WatchlistApi, WatchlistPageTemplate};
use axum::Router;
use axum::routing::{get, post};

pub struct WatchlistRoutes;

impl WatchlistRoutes {
    pub fn routes() -> Router<GameAppData> {
        Router::new()
            .route("/{lang}/watchlist", get(WatchlistPageTemplate::action))
            .route("/api/watchlist/add/{player_id}", post(WatchlistApi::add))
            .route(
                "/api/watchlist/remove/{player_id}",
                post(WatchlistApi::remove),
            )
    }
}
