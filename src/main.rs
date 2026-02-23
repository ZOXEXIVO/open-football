use std::env;
use core::utils::TimeEstimation;
use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use log::{info};
use web::{FootballSimulatorServer, GameAppData, I18nManager};
use std::sync::Arc;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() {
    color_eyre::install().unwrap();

    env_logger::Builder::from_env(Env::default()
        .default_filter_or("debug")
    ).init();

    let is_match_events_enabled = env::args().any(|arg| arg == "--match-events");
    if is_match_events_enabled {
        core::set_match_events_mode(true);
        info!("Debug mode enabled - match events will be recorded");
    }

    let is_match_recordings_disabled = env::args().any(|arg| arg == "--skip-match-recording")
        || env::var("SKIP_MATCH_RECORDING").map(|v| v == "true").unwrap_or(false);
    if is_match_recordings_disabled {
        core::set_match_recordings_mode(false);
        info!("Match recordings mode disabled");
    }

    let (database, estimated) = TimeEstimation::estimate(DatabaseLoader::load);

    info!("database loaded: {} ms", estimated);

    let game_data = DatabaseGenerator::generate(&database);

    let data = GameAppData {
        database: Arc::new(database),
        data: Arc::new(RwLock::new(Some(game_data))),
        i18n: Arc::new(I18nManager::new())
    };
    
    FootballSimulatorServer::new(data).run().await;
}
