//! Distributed match-worker support. Two roles:
//!
//! * **Coordinator** (default): starts with an empty registry; remote
//!   workers are added at runtime from the `/{lang}/workers` page, which
//!   dials the worker, runs the version-checked handshake, and registers
//!   it. A `DistributedDispatcher` is installed into `core` so
//!   `MatchPlayEnginePool` transparently offloads batches over raw TCP.
//!   The workers page renders one row per worker with live thread count,
//!   status, and stats.
//!
//! * **Worker** (`--worker --worker-port=18001`): skips DB load and
//!   the web UI; runs `WorkerServer` on the chosen port. Each accepted
//!   TCP connection speaks the framed bincode protocol — first message
//!   is a handshake, then any number of play-batch requests.
//!
//! Wire format is bincode 2 over a 4-byte length prefix. No HTTP.
//!
//! A worker records what the coordinator would have recorded — it is told on
//! the handshake — and sends each match's replay track home compressed, beside
//! the result (`recording`). Without that a remotely-played match reaches the
//! viewer with an empty recording and the page says nothing was recorded,
//! because nothing was.

pub mod dispatcher;
pub mod protocol;
pub mod recording;
pub mod registry;
pub mod server;
pub mod transport;
pub mod wire;

pub use dispatcher::DistributedDispatcher;
pub use registry::{AddWorkerOutcome, WorkerRegistry, WorkerSnapshot, WorkerStatus};
pub use server::WorkerServer;
