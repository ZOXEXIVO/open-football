#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use game_core::utils::TimeEstimation;
use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use log::info;
use web::{FootballSimulatorServer, GameAppData, I18nManager, Settings};
use web::ai::registry::{AiProviderRegistry, RegistryAiService};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

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
    game_core::ai::set_ai_service(Box::new(RegistryAiService {
        registry: Arc::clone(&ai_registry),
    }));

    let (database, estimated) = TimeEstimation::estimate(DatabaseLoader::load);

    info!("database loaded: {} ms", estimated);

    let game_data = DatabaseGenerator::generate(&database);

    let i18n = Arc::new(I18nManager::new());
    i18n.set_date(game_data.date);

    let data = GameAppData {
        database: Arc::new(database),
        data: Arc::new(RwLock::new(Some(Arc::new(game_data)))),
        process_lock: Arc::new(Mutex::new(())),
        cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        i18n,
        ai_registry,
    };

    // Open browser
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", "http://localhost:18000"])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("http://localhost:18000")
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg("http://localhost:18000")
            .spawn();
    }

    FootballSimulatorServer::new(data).run().await;
}
