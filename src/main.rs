#[cfg(target_os = "linux")]
use tikv_jemallocator::Jemalloc;
#[cfg(target_os = "windows")]
use mimalloc::MiMalloc;

// A scalable, thread-caching allocator matters more than any single hot
// path: the world sim fans out across every core and the OS heaps
// serialise concurrent alloc/free on a global lock, which becomes the
// dominant cost under that fan-out. jemalloc on Linux, mimalloc on
// Windows (the Windows system heap is the worst offender).
#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(target_os = "windows")]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

use database::{DatabaseGenerator, DatabaseLoader};
use env_logger::Env;
use log::info;
use simulator_core::r#match::MatchDispatcherRegistry;
use simulator_core::utils::TimeEstimation;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
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
