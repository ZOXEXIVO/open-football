use crate::GameAppData;
use axum::routing::{get, post};
use axum::Router;

pub fn routes() -> Router<GameAppData> {
    Router::new()
        .route(
            "/api/players/{player_id}/move-on-free",
            post(super::move_on_free_action),
        )
        .route(
            "/api/players/{player_id}/clear-unhappy",
            post(super::clear_unhappy_action),
        )
        .route(
            "/api/players/{player_id}/clear-injury",
            post(super::clear_injury_action),
        )
        .route(
            "/api/players/{player_id}/cancel-loan",
            post(super::cancel_loan_action),
        )
        .route(
            "/api/players/{player_id}/transfer",
            post(super::transfer_action),
        )
        .route(
            "/api/players/{player_id}/loan",
            post(super::loan_action),
        )
        .route(
            "/api/clubs",
            get(super::list_clubs_action),
        )
}
