use crate::GameAppData;
use axum::Router;
use axum::routing::get;

pub struct PlayerContractRoutes;

impl PlayerContractRoutes {
    pub fn create() -> Router<GameAppData> {
        Router::new().route(
            "/{lang}/players/{player_slug}/contract",
            get(super::PlayerContractPage::action),
        )
    }
}
