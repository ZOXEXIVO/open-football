use core::MatchRuntime;
use core::r#match::RecordingScope;
use log::info;
use std::env;

pub struct Settings {
    pub match_events: bool,
    pub match_recordings: bool,
    /// Keep the whole ninety minutes instead of just the goals. Off by
    /// default — see [`RecordingScope`] for what that costs.
    pub match_recordings_full: bool,
    pub match_threads: usize,
    pub match_store_threads: usize,
    /// True when the binary was invoked with `--worker`. In that mode
    /// the process skips DB load and the HTTP web UI and listens for
    /// match-batch RPCs on `worker_port`.
    pub worker_mode: bool,
    pub worker_port: u16,
}

impl Settings {
    pub fn from_env() -> Self {
        let args: Vec<String> = env::args().collect();

        let match_events = args.iter().any(|arg| arg == "--match-events");

        // On by default, and the flag is now the opt-OUT. It used to cost a
        // full ninety minutes of samples per match, which is why it was
        // opt-in; a recording is now the goals and nothing else
        // (`RecordingScope::Goals`), so there is no longer a reason to make
        // people ask for it.
        let match_recordings = !args.iter().any(|arg| arg == "--match-recording-disabled")
            && !env::var("MATCH_RECORDING_DISABLED").is_ok_and(|v| v == "true" || v == "1");

        let match_recordings_full = args.iter().any(|arg| arg == "--match-recording-full")
            || env::var("MATCH_RECORDING_FULL")
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

        Settings {
            match_events,
            match_recordings,
            match_recordings_full,
            match_threads,
            match_store_threads,
            worker_mode,
            worker_port,
        }
    }

    pub fn apply(&self) {
        MatchRuntime::set_events_mode(self.match_events);
        MatchRuntime::set_recordings_mode(self.match_recordings);
        MatchRuntime::set_recording_scope(if self.match_recordings_full {
            RecordingScope::Full
        } else {
            RecordingScope::Goals
        });
        MatchRuntime::init_engine_pool(self.match_threads);
        MatchRuntime::set_store_max_threads(self.match_store_threads);
    }

    pub fn log(&self) {
        if self.match_events {
            info!("Match events recording enabled");
        }
        if self.match_recordings {
            if self.match_recordings_full {
                info!("Match recordings enabled (full match)");
            } else {
                info!("Match recordings enabled (goals only)");
            }
        } else {
            info!("Match recordings disabled");
        }
        info!(
            "Match engine: {} threads, store: {} threads",
            self.match_threads, self.match_store_threads
        );
        if self.worker_mode {
            info!("Worker mode on, listening port {}", self.worker_port);
        }
    }
}
