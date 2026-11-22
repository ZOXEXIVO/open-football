use crate::GameAppData;
use crate::workers::{WorkersApi, WorkersPageTemplate};
use axum::Router;
use axum::routing::{get, post};

pub struct WorkersRoutes;

impl WorkersRoutes {
    pub fn routes() -> Router<GameAppData> {
        Router::new()
            .route("/{lang}/workers", get(WorkersPageTemplate::action))
            .route("/api/workers/add", post(WorkersApi::add))
            .route("/api/workers/remove", post(WorkersApi::remove))
            .route("/api/workers/status", get(WorkersApi::status))
    }
}
