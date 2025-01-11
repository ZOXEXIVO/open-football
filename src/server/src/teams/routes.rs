use crate::teams::{team_get_action, team_schedule_get_action};
use crate::GameAppData;
use axum::routing::get;
use axum::Router;

pub fn team_routes() -> Router<GameAppData> {
    Router::new()
        .route("/api/teams/{team_slug}", get(team_get_action))
        .route(
            "/api/teams/{team_slug}/schedule",
            get(team_schedule_get_action),
        )
}
