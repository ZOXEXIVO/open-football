use std::env;
use log::info;

pub struct Settings {
    pub match_events: bool,
    pub match_recordings: bool,
    pub ollama_enabled: bool,
}

impl Settings {
    pub fn from_env() -> Self {
        let args: Vec<String> = env::args().collect();

        let match_events = args.iter().any(|arg| arg == "--match-events");

        let match_recordings = !(args.iter().any(|arg| arg == "--skip-match-recording")
            || env::var("SKIP_MATCH_RECORDING")
                .map(|v| v == "true")
                .unwrap_or(false));

        let ollama_enabled = args.iter().any(|arg| arg == "--ollama")
            || env::var("OLLAMA_ENABLED")
                .map(|v| v == "true")
                .unwrap_or(false);

        Settings {
            match_events,
            match_recordings,
            ollama_enabled,
        }
    }

    pub fn apply(&self) {
        core::set_match_events_mode(self.match_events);
        core::set_match_recordings_mode(self.match_recordings);
    }

    pub fn log(&self) {
        if self.match_events {
            info!("Match events recording enabled");
        }
        if !self.match_recordings {
            info!("Match recordings mode disabled");
        }
        if self.ollama_enabled {
            info!("Ollama AI enabled");
        }
    }
}
