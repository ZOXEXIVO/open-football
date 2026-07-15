use crate::GameAppData;
use axum::Router;
use axum::routing::{get, post};

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route("/{lang}/teams/{team_slug}", get(super::team_get_action))
        .route(
            "/api/ai/team-report",
            post(super::ai_report::team_ai_report_action),
        )
}
