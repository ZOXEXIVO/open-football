use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerEventsRoutes;

impl PlayerEventsRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new()
            .route(
                "/{lang}/players/{player_slug}/events",
                get(super::PlayerEventsPage::action),
            )
            // Legacy Decisions tab — its rows are event cards now. Kept as a
            // permanent redirect so old links still resolve.
            .route(
                "/{lang}/players/{player_slug}/decisions",
                get(super::PlayerEventsPage::decisions_redirect),
            )
    }
}
