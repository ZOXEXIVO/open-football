use crate::GameAppData;
use crate::game::{GameCreate, GameProcess};
use axum::Router;
use axum::routing::{get, post};

pub struct GameRoutes;

impl GameRoutes {
    pub fn routes() -> Router<GameAppData> {
        Router::new()
            .route("/api/game/create", get(GameCreate::action))
            .route("/api/game/process", post(GameProcess::process))
            .route("/api/game/processing", get(GameProcess::status))
            .route("/api/game/cancel", post(GameProcess::cancel))
    }
}
