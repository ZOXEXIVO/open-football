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

    let is_debug = env::args().any(|arg| arg == "--debug");
    if is_debug {
        core::set_debug_mode(true);
        info!("Debug mode enabled - match events will be recorded");
    }

    let is_one_shot_game = env::var("MODE") == Ok(String::from("ONESHOT"));

    let (database, estimated) = TimeEstimation::estimate(DatabaseLoader::load);

    info!("database loaded: {} ms", estimated);

    if is_one_shot_game {
        info!("one shot game started");
    }

    let game_data = DatabaseGenerator::generate(&database);

    let data = GameAppData {
        database: Arc::new(database),
        data: Arc::new(RwLock::new(Some(game_data))),
        i18n: Arc::new(I18nManager::new()),
        is_one_shot_game,
    };
    
    FootballSimulatorServer::new(data).run().await;
}
