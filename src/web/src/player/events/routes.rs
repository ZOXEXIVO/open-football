use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route(
            "/{lang}/players/{player_slug}/events",
            get(super::player_events_action),
        )
        // Legacy Decisions tab — its rows are event cards now. Kept as a
        // permanent redirect so old links still resolve.
        .route(
            "/{lang}/players/{player_slug}/decisions",
            get(super::player_decisions_redirect),
        )
}
