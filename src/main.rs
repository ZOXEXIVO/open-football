use core::utils::TimeEstimation;
use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use log::info;
use web::{FootballSimulatorServer, GameAppData, I18nManager, Settings};
use web::ai::registry::{AiProviderRegistry, RegistryAiService};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use web::ai::providers::OllamaRequest;

#[tokio::main]
async fn main() {
    color_eyre::install().unwrap();

    env_logger::Builder::from_env(Env::default()
        .default_filter_or("debug")
    ).init();

    let settings = Settings::from_env();

    settings.apply();
    settings.log();

    let ai_registry = Arc::new(AiProviderRegistry::new());

    // ai_registry.add(
    //     "Local Ollama",
    //     Box::new(OllamaRequest::new("http://localhost", 11434, "qwen3:8b")),
    // ).await;

    // Register service so core can use it via trait — no tokio in core
    core::ai::set_ai_service(Box::new(RegistryAiService {
        registry: Arc::clone(&ai_registry),
    }));

    info!("AI registry initialized with default Local Ollama provider");

    let (database, estimated) = TimeEstimation::estimate(DatabaseLoader::load);

    info!("database loaded: {} ms", estimated);

    let game_data = DatabaseGenerator::generate(&database);

    let data = GameAppData {
        database: Arc::new(database),
        data: Arc::new(RwLock::new(Some(Arc::new(game_data)))),
        process_lock: Arc::new(Mutex::new(())),
        i18n: Arc::new(I18nManager::new()),
        ai_registry,
    };

    FootballSimulatorServer::new(data).run().await;
}
