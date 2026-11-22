use crate::GameAppData;
use axum::Router;
use axum::routing::{get, post};

pub struct PlayerActionsRoutes;

impl PlayerActionsRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new()
            .route(
                "/api/players/{player_id}/move-on-free",
                post(super::PlayerActions::move_on_free_action),
            )
            .route(
                "/api/players/{player_id}/clear-unhappy",
                post(super::PlayerActions::clear_unhappy_action),
            )
            .route(
                "/api/players/{player_id}/clear-injury",
                post(super::PlayerActions::clear_injury_action),
            )
            .route(
                "/api/players/{player_id}/toggle-force-match-selection",
                post(super::PlayerActions::toggle_force_match_selection_action),
            )
            .route(
                "/api/players/{player_id}/cancel-loan",
                post(super::PlayerActions::cancel_loan_action),
            )
            .route(
                "/api/players/{player_id}/contract-terms",
                get(super::PlayerActions::contract_terms_action),
            )
            .route(
                "/api/players/{player_id}/change-contract",
                post(super::PlayerActions::change_contract_action),
            )
            .route(
                "/api/players/{player_id}/transfer",
                post(super::PlayerActions::transfer_action),
            )
            .route(
                "/api/players/{player_id}/loan",
                post(super::PlayerActions::loan_action),
            )
            .route("/api/clubs", get(super::PlayerActions::list_clubs_action))
    }
}
