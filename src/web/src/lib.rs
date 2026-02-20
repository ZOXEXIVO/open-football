mod countries;
mod date;
mod error;
mod face;
mod game;
pub mod i18n;
mod leagues;
mod r#match;
mod player;
mod routes;
mod staff;
mod teams;
mod views;
mod common;

pub use error::{ApiError, ApiResult};
pub use i18n::{I18n, I18nManager};

use crate::routes::ServerRoutes;
use axum::response::IntoResponse;
use core::SimulatorData;
use database::DatabaseEntity;
use log::{error, info};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;

pub struct FootballSimulatorServer {
    data: GameAppData,
}

impl FootballSimulatorServer {
    pub fn new(data: GameAppData) -> Self {
        FootballSimulatorServer { data }
    }

    pub async fn run(&self) {
        let app = ServerRoutes::create()
            .layer(
                ServiceBuilder::new()
                    // Catch panics in handlers and convert them to 500 errors
                    .layer(CatchPanicLayer::custom(|_err| {
                        (
                            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error - handler panicked".to_string(),
                        ).into_response()
                    }))
            )
            .with_state(self.data.clone());

        let addr = SocketAddr::from(([0, 0, 0, 0], 18000));

        let listener = match TcpListener::bind(addr).await {
            Ok(listener) => listener,
            Err(e) => {
                error!("Failed to bind to address {}: {}", addr, e);
                panic!("Cannot start server without binding to port");
            }
        };

        info!("listen at: http://localhost:18000");

        if let Err(e) = axum::serve(listener, app).await {
            error!("Server error: {}", e);
            error!("Server stopped unexpectedly, but not crashing the process");
            // Don't panic here - just log and let the process stay alive
            // This way Docker won't restart unless the process actually exits
        }
    }
}

pub struct GameAppData {
    pub database: Arc<DatabaseEntity>,
    pub data: Arc<RwLock<Option<SimulatorData>>>,
    pub i18n: Arc<I18nManager>,
    pub is_one_shot_game: bool,
}

impl Clone for GameAppData {
    fn clone(&self) -> Self {
        GameAppData {
            database: Arc::clone(&self.database),
            data: Arc::clone(&self.data),
            i18n: Arc::clone(&self.i18n),
            is_one_shot_game: self.is_one_shot_game,
        }
    }
}
