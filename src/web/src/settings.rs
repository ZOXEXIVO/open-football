use std::env;
use log::info;

pub struct Settings {
    pub match_events: bool,
    pub match_recordings: bool,
    pub match_threads: usize,
    pub match_store_threads: usize,
}

impl Settings {
    pub fn from_env() -> Self {
        let args: Vec<String> = env::args().collect();

        let match_events = args.iter().any(|arg| arg == "--match-events");

        let match_recordings = args.iter().any(|arg| arg == "--match-recording-enabled")
            || env::var("MATCH_RECORDING_ENABLED")
                .map(|v| v == "true")
                .unwrap_or(false);

        let match_threads = args.iter()
            .find(|arg| arg.starts_with("--match-threads="))
            .and_then(|arg| arg.strip_prefix("--match-threads="))
            .and_then(|v| v.parse().ok())
            .or_else(|| env::var("MATCH_PLAY_POOL_MAX_THREADS").ok().and_then(|v| v.parse().ok()))
            .unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
            });

        let match_store_threads = env::var("MATCH_STORE_POOL_MAX_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);

        Settings {
            match_events,
            match_recordings,
            match_threads,
            match_store_threads,
        }
    }

    pub fn apply(&self) {
        core::set_match_events_mode(self.match_events);
        core::set_match_recordings_mode(self.match_recordings);
        core::init_match_engine_pool(self.match_threads);
        core::set_match_store_max_threads(self.match_store_threads);
    }

    pub fn log(&self) {
        if self.match_events {
            info!("Match events recording enabled");
        }
        if self.match_recordings {
            info!("Match recordings mode enabled");
        }
        info!("Match engine pool: {} threads", self.match_threads);
        info!("Match store pool: {} threads", self.match_store_threads);
    }
}
