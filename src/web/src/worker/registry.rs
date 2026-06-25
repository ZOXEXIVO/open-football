//! Coordinator-side state for every configured worker.
//!
//! `WorkerRegistry` owns the per-worker `TcpStream` (wrapped in a
//! `tokio::Mutex` so multiple batches assigned to the same worker run
//! serially over the framed protocol), the worker's reported metadata
//! (version, threads, computer_name, cpu_brand), the running stats,
//! and the current connection status. The dispatcher holds an
//! `Arc<WorkerRegistry>` and reads the snapshot when it routes work.

use crate::worker::protocol::{Request, Response, PROTOCOL_VERSION};
use crate::worker::transport::Frame;
use log::{info, warn};
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, RwLock};
use tokio::time::MissedTickBehavior;

/// How often the health monitor wakes. Each pass redials every
/// non-`Ready` worker (fast auto-rejoin after a drop) and actively pings
/// any stale `Ready` worker (silent-death detection). Short enough that a
/// returning worker rejoins within seconds, not the previous half-minute.
const HEALTH_INTERVAL: Duration = Duration::from_secs(5);
/// A `Ready` worker is pinged only once this long has passed since the
/// last proof of life (a completed batch or a prior pong). Workers busy
/// crunching batches refresh their liveness constantly and are never
/// pinged; only genuinely idle ones are probed.
const PING_STALE_AFTER: Duration = Duration::from_secs(15);
/// Upper bound on a liveness ping round-trip before the worker is
/// declared dead and fenced. A live worker answers a pong in well under a
/// millisecond, so this only ever fires on a wedged or vanished peer.
const PING_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Upper bound on the handshake write+read once the TCP connect has
/// succeeded. Without it a worker that accepts the socket but never
/// replies (half-open peer, host frozen right after accept) would hang
/// `add_worker` (a blocked HTTP request) and stall the health-monitor
/// loop forever. The exchange is a couple of tiny frames, so this only
/// ever fires on a genuinely unresponsive peer.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

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
    /// Last moment we had proof this worker is alive: a successful
    /// handshake, a completed batch, or a liveness pong. Drives the
    /// health monitor's ping cadence (only stale `Ready` workers are
    /// pinged) and the "last seen" readout on the workers page. `None`
    /// until the first successful contact.
    pub last_seen: Option<Instant>,
}

impl Worker {
    /// Splice a fresh handshake result into this existing slot while
    /// preserving the accumulated `stats` (batches / matches / failures)
    /// and the last-known host metadata when the redial itself failed — a
    /// worker that drops and reconnects keeps its running totals instead
    /// of resetting to zero on every blip, and its row keeps naming the
    /// machine while it's momentarily down.
    fn apply_handshake(&mut self, fresh: Worker, now: Instant) {
        let became_ready = fresh.status.is_ready();
        self.status = fresh.status;
        self.connection = fresh.connection;
        self.version = fresh.version;
        self.threads = fresh.threads;
        if !fresh.computer_name.is_empty() {
            self.computer_name = fresh.computer_name;
        }
        if !fresh.cpu_brand.is_empty() {
            self.cpu_brand = fresh.cpu_brand;
        }
        if became_ready {
            self.last_seen = Some(now);
            self.stats.last_error = None;
        }
    }
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
    /// `None` until the worker has completed at least one batch. Surfaced
    /// on the workers page as a matches-per-second readout; the dispatcher
    /// itself self-balances by greedy pull and doesn't read this. α = 0.3
    /// (see `record_batch`).
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
    /// Build an empty registry. Remote workers are added at runtime from
    /// the /workers page via [`add_worker`](Self::add_worker); until one
    /// is, the dispatcher returns `Err` for every batch and the pool
    /// falls back to local rayon. The health-monitor task is spawned here
    /// so runtime-added workers that drop are pinged and redialed
    /// automatically.
    ///
    /// Must be called from within a Tokio runtime (it spawns the
    /// health-monitor task) — that always holds for the coordinator, which
    /// constructs the registry inside `#[tokio::main]`.
    pub fn empty() -> Self {
        let registry = WorkerRegistry {
            inner: Arc::new(RwLock::new(Vec::new())),
            local_throughput_mpms: Arc::new(RwLock::new(None)),
            coordinator_version: env!("CARGO_PKG_VERSION"),
        };
        registry.spawn_health_monitor();
        registry
    }

    /// Dial a single worker at runtime, run the version-checked
    /// handshake, and splice the result into the registry. An existing
    /// entry with the same address is merged in place (stats preserved),
    /// so re-adding redials. Returns the outcome (status + reported
    /// metadata) for the web "add worker" dialog. The entry is stored
    /// regardless of outcome so a `VersionMismatch` / `Unreachable` worker
    /// shows up in the table and the health monitor keeps retrying it.
    pub async fn add_worker(&self, address: String) -> AddWorkerOutcome {
        let coordinator_version = self.coordinator_version.to_string();
        info!("worker registry: adding {}", address);
        let worker = Self::connect_and_handshake(address, coordinator_version).await;
        let outcome = AddWorkerOutcome::from_worker(&worker);
        let now = Instant::now();
        {
            let mut guard = self.inner.write().await;
            match guard.iter_mut().find(|w| w.address == worker.address) {
                // Re-adding an existing address merges in place so a
                // manual re-add keeps the worker's accumulated stats
                // (same as an automatic reconnect).
                Some(existing) => existing.apply_handshake(worker, now),
                None => guard.push(worker),
            }
        }
        outcome
    }

    /// Remove a worker from the registry by address. Dropping the slot
    /// drops its connection `Arc`, so the health monitor stops redialing
    /// it and the dispatcher stops routing new batches to it. A batch
    /// already in flight holds its own cloned connection handle and
    /// finishes normally — only future work is withheld. Returns `true`
    /// when a matching worker was found and removed.
    pub async fn remove_worker(&self, address: &str) -> bool {
        let mut guard = self.inner.write().await;
        let before = guard.len();
        guard.retain(|w| w.address != address);
        let removed = guard.len() != before;
        if removed {
            info!("worker registry: removed {}", address);
        }
        removed
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
                last_seen_secs: w.last_seen.map(|t| t.elapsed().as_secs()),
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
                    // A completed batch is the strongest proof of life —
                    // refresh it so the health monitor leaves a busy
                    // worker alone instead of pinging it.
                    w.last_seen = Some(Instant::now());
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

    fn spawn_health_monitor(&self) {
        let registry = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(HEALTH_INTERVAL);
            // A slow round (a redial can sit on CONNECT_TIMEOUT) must not
            // make ticks pile up and fire back-to-back — wait the full
            // interval after each round instead.
            ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
            ticker.tick().await; // consume the immediate first tick
            loop {
                ticker.tick().await;
                registry.health_round().await;
            }
        });
    }

    /// One health pass. Two jobs, both run concurrently and OFF the
    /// registry lock (a redial can sit on `CONNECT_TIMEOUT`, and the
    /// dispatcher needs the read lock for every batch):
    ///
    /// * **Stale `Ready` workers** (no proof of life within
    ///   `PING_STALE_AFTER`) get a liveness ping over their existing
    ///   connection. The ping uses `try_lock`, so it never contends with
    ///   an in-flight batch — a locked connection is busy, hence alive. A
    ///   failed or timed-out ping fences the worker (`Unreachable`,
    ///   connection dropped) so the next pass redials it. This is what
    ///   catches a worker whose socket died silently while the simulator
    ///   was idle — the case the old redial-only heartbeat missed, which
    ///   left a dead worker stuck `Ready` and unreconnectable.
    /// * **Every non-`Ready` worker** is redialed and re-handshaked. On
    ///   success its slot flips back to `Ready` with accumulated stats
    ///   preserved, so a worker that dropped and came back rejoins the
    ///   pool automatically — no operator action, no failed batch needed.
    ///
    /// Results are spliced back in by address (not index) so a concurrent
    /// add/redial can't write the wrong slot.
    async fn health_round(&self) {
        let now = Instant::now();
        let probes: Vec<Probe> = {
            let guard = self.inner.read().await;
            guard
                .iter()
                .filter_map(|w| match (&w.status, &w.connection) {
                    (WorkerStatus::Ready, Some(conn)) => {
                        let stale = w
                            .last_seen
                            .map_or(true, |t| now.duration_since(t) >= PING_STALE_AFTER);
                        stale.then(|| Probe::Ping {
                            address: w.address.clone(),
                            conn: Arc::clone(conn),
                        })
                    }
                    _ => Some(Probe::Redial {
                        address: w.address.clone(),
                    }),
                })
                .collect()
        };
        if probes.is_empty() {
            return;
        }

        let coordinator_version = self.coordinator_version.to_string();
        let handles: Vec<_> = probes
            .into_iter()
            .map(|probe| {
                let v = coordinator_version.clone();
                tokio::spawn(async move {
                    match probe {
                        Probe::Ping { address, conn } => ProbeResult::Ping {
                            address,
                            outcome: Self::ping_worker(&conn).await,
                        },
                        Probe::Redial { address } => {
                            let worker = Self::connect_and_handshake(address.clone(), v).await;
                            ProbeResult::Redial { address, worker }
                        }
                    }
                })
            })
            .collect();

        let mut results = Vec::with_capacity(handles.len());
        for h in handles {
            if let Ok(r) = h.await {
                results.push(r);
            }
        }

        let now = Instant::now();
        let mut guard = self.inner.write().await;
        for result in results {
            match result {
                ProbeResult::Ping { address, outcome } => {
                    let Some(w) = guard.iter_mut().find(|w| w.address == address) else {
                        continue;
                    };
                    match outcome {
                        PingOutcome::Pong => {
                            w.last_seen = Some(now);
                            w.stats.last_error = None;
                        }
                        // Busy with a batch ⇒ provably alive; nothing to do.
                        PingOutcome::Busy => {}
                        PingOutcome::Dead(reason) => {
                            // Only fence it if it's still the Ready entry we
                            // pinged — a concurrent reconnect may already
                            // have refreshed this slot.
                            if w.status.is_ready() {
                                warn!("worker {}: ping failed — {}; fencing", address, reason);
                                w.stats.last_error = Some(reason.clone());
                                w.status = WorkerStatus::Unreachable { reason };
                                w.connection = None;
                            }
                        }
                    }
                }
                ProbeResult::Redial { address, worker } => {
                    if let Some(w) = guard.iter_mut().find(|w| w.address == address) {
                        // Don't clobber a worker that became Ready meanwhile
                        // (e.g. a manual re-add landed during this round).
                        if !w.status.is_ready() {
                            w.apply_handshake(worker, now);
                        }
                    }
                }
            }
        }
    }

    /// Send one `Ping` over an idle worker connection and wait for the
    /// `Pong`, bounded by `PING_TIMEOUT`. `try_lock` keeps this off any
    /// connection a batch is currently using: a locked connection means
    /// the worker is busy, which is itself proof of life, so we report
    /// `Busy` and leave the slot untouched. Because the dispatcher holds
    /// the same `Mutex` for a whole batch round-trip, a ping and a batch
    /// can never interleave on the wire.
    async fn ping_worker(conn: &Arc<Mutex<TcpStream>>) -> PingOutcome {
        let mut stream = match conn.try_lock() {
            Ok(s) => s,
            Err(_) => return PingOutcome::Busy,
        };
        let exchange = async {
            Frame::write(&mut *stream, &Request::Ping).await?;
            Frame::read::<Response>(&mut *stream).await
        };
        match tokio::time::timeout(PING_TIMEOUT, exchange).await {
            Ok(Ok(Response::Pong)) => PingOutcome::Pong,
            Ok(Ok(other)) => PingOutcome::Dead(format!(
                "unexpected ping reply: {:?}",
                std::mem::discriminant(&other)
            )),
            Ok(Err(e)) => PingOutcome::Dead(format!("ping io: {}", e)),
            Err(_) => PingOutcome::Dead("ping timeout".to_string()),
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
        let exchange = async {
            Frame::write(&mut stream, &handshake).await?;
            Frame::read(&mut stream).await
        };
        let response: Response = match tokio::time::timeout(HANDSHAKE_TIMEOUT, exchange).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => return Self::unreachable(address, format!("handshake io: {}", e)),
            Err(_) => return Self::unreachable(address, "handshake timeout".to_string()),
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
                        last_seen: None,
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
                        last_seen: None,
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
                    last_seen: Some(Instant::now()),
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
            last_seen: None,
        }
    }
}

/// One scheduled probe in a [`WorkerRegistry::health_round`]: either ping
/// an idle `Ready` worker over its live connection, or redial a non-`Ready`
/// one. Built under the read lock, then executed concurrently off-lock.
enum Probe {
    Ping {
        address: String,
        conn: Arc<Mutex<TcpStream>>,
    },
    Redial {
        address: String,
    },
}

/// The result of running a [`Probe`], applied back into the registry by
/// address once the whole round has finished.
enum ProbeResult {
    Ping { address: String, outcome: PingOutcome },
    Redial { address: String, worker: Worker },
}

/// Outcome of a single liveness ping.
enum PingOutcome {
    /// Worker answered the ping — alive.
    Pong,
    /// Connection was locked by an in-flight batch — alive, skip.
    Busy,
    /// No (valid) reply within the timeout — fence the worker.
    Dead(String),
}

pub struct WorkerSnapshot {
    pub address: String,
    pub status: WorkerStatus,
    pub version: String,
    pub threads: usize,
    pub computer_name: String,
    pub cpu_brand: String,
    pub stats: WorkerStats,
    /// Seconds since the last proof of life, or `None` if never contacted.
    /// Rendered as a relative "last seen" age on the workers page.
    pub last_seen_secs: Option<u64>,
}

/// Outcome of a runtime [`WorkerRegistry::add_worker`] call, serialized
/// straight back to the web "add worker" dialog as JSON. `status` is one
/// of `ready` / `version_mismatch` / `unreachable`; `detail` carries the
/// mismatch version or the connection-failure reason when relevant.
#[derive(Debug, Clone, Serialize)]
pub struct AddWorkerOutcome {
    pub status: &'static str,
    pub address: String,
    pub version: String,
    pub threads: usize,
    pub computer_name: String,
    pub cpu_brand: String,
    pub detail: String,
}

impl AddWorkerOutcome {
    fn from_worker(w: &Worker) -> Self {
        let (status, detail) = match &w.status {
            WorkerStatus::Ready => ("ready", String::new()),
            WorkerStatus::VersionMismatch { worker_version } => {
                ("version_mismatch", worker_version.clone())
            }
            WorkerStatus::Unreachable { reason } => ("unreachable", reason.clone()),
            WorkerStatus::Connecting => ("connecting", String::new()),
        };
        AddWorkerOutcome {
            status,
            address: w.address.clone(),
            version: w.version.clone(),
            threads: w.threads,
            computer_name: w.computer_name.clone(),
            cpu_brand: w.cpu_brand.clone(),
            detail,
        }
    }
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
