//! Coordinator-side state for every configured worker.
//!
//! `WorkerRegistry` owns the per-worker `TcpStream` (wrapped in a
//! `tokio::Mutex` so multiple batches assigned to the same worker run
//! serially over the framed protocol), the worker's reported metadata
//! (version, threads, computer_name, cpu_brand), the running stats,
//! and the current connection status. The dispatcher holds an
//! `Arc<WorkerRegistry>` and reads the snapshot when it routes work.

use crate::worker::config::WorkersConfig;
use crate::worker::protocol::{Request, Response, PROTOCOL_VERSION};
use crate::worker::transport::Frame;
use log::{info, warn};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// One worker as seen by the coordinator. Holds a single TCP
/// connection to the worker process — the worker itself runs incoming
/// batches in parallel on its own rayon pool, so one pipe is enough
/// to keep it busy. `connection` is `None` whenever `status` isn't
/// `Ready`.
pub struct Worker {
    pub address: String,
    pub status: WorkerStatus,
    pub version: String,
    pub threads: usize,
    pub computer_name: String,
    pub cpu_brand: String,
    pub stats: WorkerStats,
    pub connection: Option<Arc<Mutex<TcpStream>>>,
}

#[derive(Debug, Clone)]
pub enum WorkerStatus {
    Connecting,
    Ready,
    VersionMismatch { worker_version: String },
    Unreachable { reason: String },
}

impl WorkerStatus {
    pub fn is_ready(&self) -> bool {
        matches!(self, WorkerStatus::Ready)
    }

    pub fn label(&self) -> &'static str {
        match self {
            WorkerStatus::Connecting => "connecting",
            WorkerStatus::Ready => "ready",
            WorkerStatus::VersionMismatch { .. } => "version_mismatch",
            WorkerStatus::Unreachable { .. } => "unreachable",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkerStats {
    pub batches_sent: u64,
    pub matches_completed: u64,
    pub failures: u64,
    pub last_latency_ms: Option<u64>,
    pub last_error: Option<String>,
    /// EWMA of `matches / latency_ms` observed on successful batches.
    /// `None` until the worker has completed at least one batch — the
    /// dispatcher's `build_plan` falls back to thread-count weighting
    /// for unmeasured workers. α = 0.3 (see `update_throughput`).
    pub throughput_mpms: Option<f64>,
}

/// EWMA smoothing factor for per-worker throughput. Low enough to ride
/// out a single anomalous slow batch (network blip, contended host),
/// high enough to reflect a genuine speed change within ~5 batches.
const THROUGHPUT_ALPHA: f64 = 0.3;

#[derive(Clone)]
pub struct WorkerRegistry {
    inner: Arc<RwLock<Vec<Worker>>>,
    /// Throughput estimate for the coordinator's own local rayon slot —
    /// updated from the dispatcher every time a local chunk finishes.
    /// Held separately because the local target isn't a `Worker`.
    local_throughput_mpms: Arc<RwLock<Option<f64>>>,
    coordinator_version: &'static str,
}

impl WorkerRegistry {
    /// Build an empty registry — used when no `open-football.conf` is
    /// present. The dispatcher returns `Err` for every batch in that
    /// state and the pool falls back to local rayon.
    pub fn empty() -> Self {
        WorkerRegistry {
            inner: Arc::new(RwLock::new(Vec::new())),
            local_throughput_mpms: Arc::new(RwLock::new(None)),
            coordinator_version: env!("CARGO_PKG_VERSION"),
        }
    }

    /// Dial every worker in `cfg` in parallel and run the handshake.
    /// Returns a populated registry — entries that failed are still
    /// present (with `Unreachable` or `VersionMismatch` status) so the
    /// workers page can surface the failure.
    pub async fn from_config(cfg: WorkersConfig) -> Self {
        let registry = Self::empty();
        let coordinator_version = registry.coordinator_version.to_string();

        info!("worker registry: {} address(es)", cfg.workers.len());

        let handles: Vec<_> = cfg
            .workers
            .into_iter()
            .map(|entry| {
                let v = coordinator_version.clone();
                tokio::spawn(async move { Self::connect_and_handshake(entry.address, v).await })
            })
            .collect();

        let mut workers = Vec::with_capacity(handles.len());
        for h in handles {
            if let Ok(worker) = h.await {
                workers.push(worker);
            }
        }

        {
            let mut guard = registry.inner.write().await;
            *guard = workers;
        }

        registry.spawn_heartbeat();
        registry
    }

    /// Read-only snapshot of every worker, in config order. Cheap to
    /// build (clones the small per-worker metadata; the connection
    /// `Arc` stays shared). Used by the home page renderer.
    pub async fn snapshot(&self) -> Vec<WorkerSnapshot> {
        let guard = self.inner.read().await;
        guard
            .iter()
            .map(|w| WorkerSnapshot {
                address: w.address.clone(),
                status: w.status.clone(),
                version: w.version.clone(),
                threads: w.threads,
                computer_name: w.computer_name.clone(),
                cpu_brand: w.cpu_brand.clone(),
                stats: w.stats.clone(),
            })
            .collect()
    }

    /// Snapshot of just the `Ready` workers' (address, threads,
    /// connection handle). Used by the dispatcher to pick a target
    /// for a league batch.
    pub async fn ready_handles(&self) -> Vec<ReadyWorker> {
        let guard = self.inner.read().await;
        guard
            .iter()
            .filter_map(|w| match (&w.status, &w.connection) {
                (WorkerStatus::Ready, Some(c)) => Some(ReadyWorker {
                    address: w.address.clone(),
                    threads: w.threads,
                    throughput_mpms: w.stats.throughput_mpms,
                    connection: Arc::clone(c),
                }),
                _ => None,
            })
            .collect()
    }

    /// Snapshot of the local rayon slot's current throughput estimate —
    /// `None` until the dispatcher has finished at least one local
    /// chunk. The dispatcher reads this when building a plan so the
    /// local slot is weighted against remote workers on the same
    /// matches-per-ms scale.
    pub async fn local_throughput(&self) -> Option<f64> {
        *self.local_throughput_mpms.read().await
    }

    /// Update the local-slot throughput EWMA after a local chunk
    /// finishes. Called from the dispatcher's `Target::Local` branch.
    /// First sample seeds the value directly; subsequent samples smooth
    /// with `THROUGHPUT_ALPHA` so a transient blip doesn't dominate.
    ///
    /// Takes a `Duration` rather than an integer ms because a hot
    /// rayon pool routinely finishes a chunk in under 1 ms; flooring
    /// to whole milliseconds would zero out the sample and the
    /// `latency == 0` guard would then suppress the recording forever
    /// — that's why the workers page showed `—` for the local row.
    pub async fn record_local_batch(&self, matches: usize, latency: Duration) {
        if matches == 0 {
            return;
        }
        let latency_ms = latency.as_secs_f64() * 1_000.0;
        if latency_ms <= 0.0 {
            return;
        }
        let observed = matches as f64 / latency_ms;
        let mut guard = self.local_throughput_mpms.write().await;
        *guard = Some(match *guard {
            None => observed,
            Some(prev) => THROUGHPUT_ALPHA * observed + (1.0 - THROUGHPUT_ALPHA) * prev,
        });
    }

    pub async fn record_batch(
        &self,
        address: &str,
        matches: usize,
        latency_ms: u64,
        outcome: BatchOutcome,
    ) {
        let mut guard = self.inner.write().await;
        if let Some(w) = guard.iter_mut().find(|w| w.address == address) {
            w.stats.batches_sent = w.stats.batches_sent.saturating_add(1);
            w.stats.last_latency_ms = Some(latency_ms);
            match outcome {
                BatchOutcome::Ok => {
                    w.stats.matches_completed =
                        w.stats.matches_completed.saturating_add(matches as u64);
                    w.stats.last_error = None;
                    if matches > 0 && latency_ms > 0 {
                        let observed = matches as f64 / latency_ms as f64;
                        w.stats.throughput_mpms = Some(match w.stats.throughput_mpms {
                            None => observed,
                            Some(prev) => {
                                THROUGHPUT_ALPHA * observed + (1.0 - THROUGHPUT_ALPHA) * prev
                            }
                        });
                    }
                }
                BatchOutcome::Failed(reason) => {
                    w.stats.failures = w.stats.failures.saturating_add(1);
                    w.stats.last_error = Some(reason);
                    w.status = WorkerStatus::Unreachable {
                        reason: w
                            .stats
                            .last_error
                            .clone()
                            .unwrap_or_else(|| "unknown".to_string()),
                    };
                    w.connection = None;
                }
            }
        }
    }

    fn spawn_heartbeat(&self) {
        let registry = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
            ticker.tick().await; // skip the immediate first tick
            loop {
                ticker.tick().await;
                registry.heartbeat_round().await;
            }
        });
    }

    /// One heartbeat pass: redial every non-Ready entry and re-handshake.
    /// Ready entries are left alone so we don't disturb an in-flight batch.
    async fn heartbeat_round(&self) {
        let to_dial: Vec<(usize, String)> = {
            let guard = self.inner.read().await;
            guard
                .iter()
                .enumerate()
                .filter(|(_, w)| !w.status.is_ready())
                .map(|(i, w)| (i, w.address.clone()))
                .collect()
        };
        if to_dial.is_empty() {
            return;
        }
        let coordinator_version = self.coordinator_version.to_string();
        let handles: Vec<_> = to_dial
            .into_iter()
            .map(|(idx, addr)| {
                let v = coordinator_version.clone();
                tokio::spawn(async move {
                    let worker = Self::connect_and_handshake(addr, v).await;
                    (idx, worker)
                })
            })
            .collect();
        // Drain handles WITHOUT holding the write lock — each
        // reconnect attempt can sit on CONNECT_TIMEOUT (5 s), and the
        // dispatcher's `ready_handles().await` needs the read lock for
        // every dispatched batch. Take the lock only when we have the
        // results in hand and splice them in.
        let mut results = Vec::with_capacity(handles.len());
        for h in handles {
            if let Ok(pair) = h.await {
                results.push(pair);
            }
        }
        let mut guard = self.inner.write().await;
        for (idx, fresh) in results {
            if let Some(slot) = guard.get_mut(idx) {
                *slot = fresh;
            }
        }
    }

    async fn connect_and_handshake(address: String, coordinator_version: String) -> Worker {
        let connect = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&address)).await;
        let mut stream = match connect {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Self::unreachable(address, e.to_string()),
            Err(_) => return Self::unreachable(address, "connect timeout".to_string()),
        };

        let handshake = Request::Handshake {
            coordinator_version: coordinator_version.clone(),
            protocol_version: PROTOCOL_VERSION,
        };
        if let Err(e) = Frame::write(&mut stream, &handshake).await {
            return Self::unreachable(address, format!("handshake write: {}", e));
        }
        let response: Response = match Frame::read(&mut stream).await {
            Ok(r) => r,
            Err(e) => return Self::unreachable(address, format!("handshake read: {}", e)),
        };

        match response {
            Response::Handshake {
                version,
                protocol_version,
                threads,
                computer_name,
                cpu_brand,
            } => {
                if version != coordinator_version {
                    warn!(
                        "worker {}: version mismatch (coordinator {}, worker {}) — skipping",
                        address, coordinator_version, version
                    );
                    return Worker {
                        address,
                        status: WorkerStatus::VersionMismatch {
                            worker_version: version.clone(),
                        },
                        version,
                        threads,
                        computer_name,
                        cpu_brand,
                        stats: WorkerStats::default(),
                        connection: None,
                    };
                }
                if protocol_version != PROTOCOL_VERSION {
                    warn!(
                        "worker {}: protocol mismatch (coordinator {}, worker {}) — skipping",
                        address, PROTOCOL_VERSION, protocol_version
                    );
                    return Worker {
                        address,
                        status: WorkerStatus::VersionMismatch {
                            worker_version: format!(
                                "{} (protocol {} ≠ {})",
                                version, protocol_version, PROTOCOL_VERSION
                            ),
                        },
                        version,
                        threads,
                        computer_name,
                        cpu_brand,
                        stats: WorkerStats::default(),
                        connection: None,
                    };
                }
                info!(
                    "worker {}: ready — {} threads, v{}, host {}",
                    address, threads, version, computer_name
                );
                Worker {
                    address,
                    status: WorkerStatus::Ready,
                    version,
                    threads,
                    computer_name,
                    cpu_brand,
                    stats: WorkerStats::default(),
                    connection: Some(Arc::new(Mutex::new(stream))),
                }
            }
            Response::HandshakeRejected { reason } => {
                warn!("worker {}: rejected handshake — {}", address, reason);
                Self::unreachable(address, format!("rejected: {}", reason))
            }
            other => Self::unreachable(
                address,
                format!("unexpected handshake reply: {:?}", std::mem::discriminant(&other)),
            ),
        }
    }

    fn unreachable(address: String, reason: String) -> Worker {
        Worker {
            address,
            status: WorkerStatus::Unreachable { reason },
            version: String::new(),
            threads: 0,
            computer_name: String::new(),
            cpu_brand: String::new(),
            stats: WorkerStats::default(),
            connection: None,
        }
    }
}

pub struct WorkerSnapshot {
    pub address: String,
    pub status: WorkerStatus,
    pub version: String,
    pub threads: usize,
    pub computer_name: String,
    pub cpu_brand: String,
    pub stats: WorkerStats,
}

impl WorkerSnapshot {
    /// Render-friendly matches-per-second from the EWMA throughput.
    /// Returns `None` until the worker has completed at least one batch.
    pub fn throughput_mps(&self) -> Option<f64> {
        self.stats.throughput_mpms.map(|mpms| mpms * 1000.0)
    }
}

#[derive(Clone)]
pub struct ReadyWorker {
    pub address: String,
    pub threads: usize,
    /// EWMA throughput from the registry — `None` until the worker has
    /// completed at least one batch. The dispatcher's `build_plan`
    /// falls back to a thread-count seed weight when this is `None`.
    pub throughput_mpms: Option<f64>,
    pub connection: Arc<Mutex<TcpStream>>,
}

pub enum BatchOutcome {
    Ok,
    Failed(String),
}

/// Latency timer convenience — caller writes
/// `let _t = LatencyTimer::start(); ... let ms = _t.elapsed_ms();`
pub struct LatencyTimer(Instant);

impl LatencyTimer {
    pub fn start() -> Self {
        LatencyTimer(Instant::now())
    }
    pub fn elapsed(&self) -> Duration {
        self.0.elapsed()
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.0.elapsed().as_millis() as u64
    }
}
