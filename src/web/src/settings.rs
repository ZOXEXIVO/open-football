use core::MatchRuntime;
use log::info;
use std::env;

pub struct Settings {
    pub match_events: bool,
    pub match_recordings: bool,
    pub match_threads: usize,
    pub match_store_threads: usize,
    /// True when the binary was invoked with `--worker`. In that mode
    /// the process skips DB load and the HTTP web UI and listens for
    /// match-batch RPCs on `worker_port`.
    pub worker_mode: bool,
    pub worker_port: u16,
    /// Path to the TOML workers list (coordinator-side). Defaults to
    /// `./open-football.conf`. Ignored when `worker_mode == true`.
    pub workers_config_path: String,
}

impl Settings {
    pub fn from_env() -> Self {
        let args: Vec<String> = env::args().collect();

        let match_events = args.iter().any(|arg| arg == "--match-events");

        let match_recordings = args.iter().any(|arg| arg == "--match-recording-enabled")
            || env::var("MATCH_RECORDING_ENABLED")
                .map(|v| v == "true")
                .unwrap_or(false);

        let match_threads = args
            .iter()
            .find(|arg| arg.starts_with("--match-threads="))
            .and_then(|arg| arg.strip_prefix("--match-threads="))
            .and_then(|v| v.parse().ok())
            .or_else(|| {
                env::var("MATCH_PLAY_POOL_MAX_THREADS")
                    .ok()
                    .and_then(|v| v.parse().ok())
            })
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
            });

        let match_store_threads = env::var("MATCH_STORE_POOL_MAX_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);

        let worker_mode = args.iter().any(|arg| arg == "--worker");

        let worker_port = args
            .iter()
            .find(|arg| arg.starts_with("--worker-port="))
            .and_then(|arg| arg.strip_prefix("--worker-port="))
            .and_then(|v| v.parse().ok())
            .unwrap_or(18001);

        let workers_config_path = args
            .iter()
            .find(|arg| arg.starts_with("--workers-config="))
            .and_then(|arg| arg.strip_prefix("--workers-config="))
            .map(|s| s.to_string())
            .unwrap_or_else(|| "open-football.conf".to_string());

        Settings {
            match_events,
            match_recordings,
            match_threads,
            match_store_threads,
            worker_mode,
            worker_port,
            workers_config_path,
        }
    }

    pub fn apply(&self) {
        MatchRuntime::set_events_mode(self.match_events);
        MatchRuntime::set_recordings_mode(self.match_recordings);
        MatchRuntime::init_engine_pool(self.match_threads);
        MatchRuntime::set_store_max_threads(self.match_store_threads);
    }

    pub fn log(&self) {
        if self.match_events {
            info!("Match events recording enabled");
        }
        if self.match_recordings {
            info!("Match recordings mode enabled");
        }
        info!(
            "Match engine: {} threads, store: {} threads",
            self.match_threads, self.match_store_threads
        );
        if self.worker_mode {
            info!("Worker mode on, listening port {}", self.worker_port);
        } else {
            info!("Workers config: {}", self.workers_config_path);
        }
    }
}
