#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use log::info;
use simulator_core::r#match::MatchDispatcherRegistry;
use simulator_core::utils::TimeEstimation;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use web::ai::registry::{AiProviderRegistry, RegistryAiService};
use web::{
    DistributedDispatcher, FootballSimulatorServer, GameAppData, I18nManager, Settings,
    WorkerRegistry, WorkerServer,
};

#[tokio::main]
async fn main() {
    color_eyre::install().unwrap();

    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    info!("SIMD: {}", simulator_core::utils::cpu::simd_kernel_name());

    let settings = Settings::from_env();

    settings.apply();
    settings.log();

    // Worker mode: skip DB load + UI, just serve match RPCs.
    if settings.worker_mode {
        WorkerServer::new(settings.worker_port).run().await;
        return;
    }

    let ai_registry = Arc::new(AiProviderRegistry::new());

    // ai_registry.add(
    //     "Local Ollama",
    //     Box::new(OllamaRequest::new("http://localhost", 11434, "qwen3.6:35b")),
    // ).await;

    // Register service so core can use it via trait — no tokio in core
    simulator_core::ai::AiServiceRegistry::set(Box::new(RegistryAiService {
        registry: Arc::clone(&ai_registry),
    }));

    // Start with an empty worker registry — remote workers are added at
    // runtime from the /workers page. While the registry is empty the
    // dispatcher returns `Err` for every batch and the pool falls back
    // to the local rayon path.
    let workers = WorkerRegistry::empty();

    // Install the dispatcher into core. The pool will use it for every
    // batch from here on; empty registry → Err → local rayon fallback.
    // `local_threads` lets the coordinator host participate as a virtual
    // worker so its CPU isn't idle while remote workers crunch. We use
    // the same match_threads value as the local rayon pool.
    MatchDispatcherRegistry::set(Box::new(DistributedDispatcher::new(
        workers.clone(),
        tokio::runtime::Handle::current(),
        settings.match_threads,
    )));

    let (database, estimated) = TimeEstimation::estimate(DatabaseLoader::load);

    let (game_data, gen_ms) = TimeEstimation::estimate(|| DatabaseGenerator::generate(&database));

    info!(
        "database loaded: {} ms, generated: {} ms",
        estimated, gen_ms
    );

    let i18n = Arc::new(I18nManager::new());
    i18n.set_date(game_data.date);

    let data = GameAppData {
        database: Arc::new(database),
        data: Arc::new(RwLock::new(Some(Arc::new(game_data)))),
        process_lock: Arc::new(Mutex::new(())),
        cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        i18n,
        ai_registry,
        workers,
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
