mod countries;
mod date;
mod game;
mod leagues;
mod r#match;
mod player;
mod routes;
mod teams;

use crate::routes::ServerRoutes;
use core::SimulatorData;
use database::DatabaseEntity;
use log::info;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;

pub struct FootballSimulatorServer {
    data: GameAppData,
}

impl FootballSimulatorServer {
    pub fn new(data: GameAppData) -> Self {
        FootballSimulatorServer { data }
    }

    pub async fn run(&self) {
        let app = ServerRoutes::create().with_state(self.data.clone());

        let addr = SocketAddr::from(([0, 0, 0, 0], 18000));

        let listener = TcpListener::bind(addr).await.unwrap();

        info!("listen at: http://localhost:18000");

        axum::serve(listener, app)
            .await
            .expect("failed to start server");
    }
}

pub struct GameAppData {
    pub database: Arc<DatabaseEntity>,
    pub data: Arc<RwLock<Option<SimulatorData>>>,
}

impl Clone for GameAppData {
    fn clone(&self) -> Self {
        GameAppData {
            database: Arc::clone(&self.database),
            data: Arc::clone(&self.data),
        }
    }
}
