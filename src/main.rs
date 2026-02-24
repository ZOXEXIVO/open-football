use core::utils::TimeEstimation;
use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use log::info;
use web::{FootballSimulatorServer, GameAppData, I18nManager, Settings};
use std::sync::Arc;
use tokio::sync::RwLock;
use web::ollama::OllamaRequest;

#[tokio::main]
async fn main() {
    color_eyre::install().unwrap();

    env_logger::Builder::from_env(Env::default()
        .default_filter_or("info")
    ).init();

    let settings = Settings::from_env();

    settings.apply();
    settings.log();

    if settings.ollama_enabled {
        let ai = OllamaRequest::from_env();
        core::ai::set_ai(Box::new(ai));
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
