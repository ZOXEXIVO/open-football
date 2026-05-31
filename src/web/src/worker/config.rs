//! TOML loader for the coordinator-side worker list. Reads
//! `open-football.conf` from the working directory by default; the
//! `--workers-config=<path>` CLI flag overrides the path.
//!
//! File schema:
//!
//! ```toml
//! [[worker]]
//! address = "10.0.0.5:18001"
//!
//! [[worker]]
//! address = "10.0.0.6:18001"
//! ```
//!
//! Missing file ⇒ `Ok(None)`. Empty `worker` list ⇒ `Ok(Some(empty))`,
//! which still lets the coordinator log "no remote workers configured"
//! at startup. Malformed file ⇒ `Err`.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkersConfig {
    #[serde(default, rename = "worker")]
    pub workers: Vec<WorkerEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerEntry {
    /// `host:port` form. Resolved via `TcpStream::connect`, so DNS
    /// names work.
    pub address: String,
}

impl WorkersConfig {
    /// Load the config from the given path. Returns `Ok(None)` when
    /// the file is missing — that's the "no remote workers" default —
    /// and `Err` for read or parse failures so the operator sees the
    /// problem at startup.
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Option<Self>> {
        let path = path.as_ref();
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let cfg: Self = toml::from_str(&text)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(Some(cfg))
    }
}
