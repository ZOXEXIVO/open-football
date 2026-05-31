//! Distributed match-worker support. Two roles:
//!
//! * **Coordinator** (default): reads `open-football.conf`, dials every
//!   listed worker, runs the version-checked handshake, and installs a
//!   `DistributedDispatcher` into `core` so `MatchPlayEnginePool`
//!   transparently offloads batches over raw TCP. The home page
//!   (`/{lang}/countries`) renders one card per worker with live
//!   thread count, status, and stats.
//!
//! * **Worker** (`--worker --worker-port=18001`): skips DB load and
//!   the web UI; runs `WorkerServer` on the chosen port. Each accepted
//!   TCP connection speaks the framed bincode protocol — first message
//!   is a handshake, then any number of play-batch requests.
//!
//! Wire format is bincode 2 over a 4-byte length prefix. No HTTP.

pub mod config;
pub mod dispatcher;
pub mod protocol;
pub mod registry;
pub mod server;
pub mod transport;
pub mod wire;

pub use config::WorkersConfig;
pub use dispatcher::DistributedDispatcher;
pub use registry::{WorkerRegistry, WorkerSnapshot, WorkerStatus};
pub use server::WorkerServer;
