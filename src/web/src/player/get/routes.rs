use crate::GameAppData;
use axum::Router;
use axum::routing::{get, post};

pub struct PlayerGetRoutes;

impl PlayerGetRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new()
            .route(
                "/{lang}/players/{player_slug}",
                get(super::PlayerGetPage::action),
            )
            .route(
                "/api/ai/player-report",
                post(super::ai_report::PlayerAiReport::action),
            )
    }
}
