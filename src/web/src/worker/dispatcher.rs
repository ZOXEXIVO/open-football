//! Coordinator-side `MatchDispatcher` impl. A dispatch spreads its batch
//! across every Ready worker (plus the local rayon pool) using a shared
//! work queue, so a single matchday never sits on one worker while the
//! rest of the fleet idles — and a worker dying mid-batch never stalls or
//! drops the run.
//!
//! Routing model (work-stealing):
//!
//!   1. Snapshot the `Ready` worker slots and prepend a virtual `Local`
//!      target (skipped when `local_threads = 0`). Each slot carries the
//!      chunk size it pulls per round: `2 × threads`, large enough to
//!      keep that worker's rayon pool busy without a long straggler tail
//!      on the chunk's own internal parallelism.
//!   2. Seed one shared queue with every match index and spawn one task
//!      per slot. Each task greedily pulls a chunk off the queue, runs
//!      it, records the result, and loops until the queue drains. Greedy
//!      pull self-balances: a faster slot comes back for more sooner, so
//!      throughput — not a static weight — decides how much each slot
//!      does.
//!   3. **Failure handling.** When a remote chunk fails (I/O error,
//!      error response, or shape mismatch) the worker is marked
//!      `Unreachable` and its connection dropped (so it leaves the ready
//!      set for the next dispatch), the in-flight chunk is handed back to
//!      the FRONT of the queue for a healthy slot to retry, and that slot
//!      stops pulling — a dead worker is fenced off and never touched
//!      again for the rest of the batch. The other slots keep draining
//!      the queue, including the requeued chunk.
//!   4. **Safety net.** After all slot tasks finish, any match still
//!      unprocessed (every slot that could have run it failed, or a task
//!      panicked) is played on the local rayon pool. A total worker
//!      wipeout degrades to local-only; the caller always gets a full,
//!      correctly-sized result set.
//!
//! Fast path: a batch that fits in a single local chunk skips every
//! remote slot — the network round-trip would cost more than the local
//! pool takes to play it.
//!
//! Local fallback: with no Ready targets and no local share the
//! dispatcher returns `Err` so the engine pool runs the rayon path on the
//! unmodified input.

use crate::worker::protocol::{MatchEnvelope, MatchOutcome, Request, Response};
use crate::worker::registry::{BatchOutcome, LatencyTimer, ReadyWorker, WorkerRegistry};
use crate::worker::transport::Frame;
use crate::worker::wire::{LeagueMatchWire, SquadFixtureWire, SquadWire};
use core::MatchRuntime;
use core::r#match::{Match, MatchDispatcher, MatchResult, MatchResultRaw, MatchSquad, Score};
use log::{info, warn};
use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::runtime::Handle;
use tokio::sync::Mutex;

/// Upper bound on a single remote batch round-trip (request write +
/// response read). A stopped or wedged worker that never delivers a
/// FIN/RST — host slept, cable pulled, process paused, or simply hung —
/// would otherwise leave `Frame::read` blocked forever. That matters
/// here far more than for a typical client: the dispatch slot task is
/// awaited synchronously by the matchday's rayon thread (which is itself
/// inside `block_on`), so one unresponsive worker freezes the entire
/// simulation with no way to cancel mid-day. The timeout converts that
/// unbounded hang into an ordinary batch failure the fence/requeue/
/// safety-net path already handles: the worker is marked Unreachable,
/// its chunk retried on a healthy slot, and processing continues.
///
/// One minute — far above the realistic worst case (chunks are only
/// `2 × threads` fast matches, which a live worker answers in well under
/// a second even on modest hardware) so a slow-but-alive worker is never
/// fenced by accident. A false fence is self-healing anyway: its chunk
/// runs elsewhere and the health monitor redials it within `HEALTH_INTERVAL`.
const REMOTE_BATCH_TIMEOUT: Duration = Duration::from_secs(60);

pub struct DistributedDispatcher {
    registry: WorkerRegistry,
    runtime: Handle,
    /// `0` disables the coordinator's local share — every chunk is routed
    /// to a remote worker (or the rayon-fallback path when no workers are
    /// Ready). Otherwise the local rayon pool participates as a virtual
    /// slot so the coordinator's CPU isn't idle while workers crunch.
    local_threads: usize,
}

enum Target {
    Local,
    Remote(ReadyWorker),
}

impl Target {
    fn label(&self) -> &str {
        match self {
            Target::Local => "local",
            Target::Remote(w) => &w.address,
        }
    }
}

/// One dispatch target plus the chunk size it pulls per round. Chunks are
/// sized at `2 × threads` so each round-trip carries enough work to keep
/// the target's rayon pool fully occupied.
struct Slot {
    target: Target,
    chunk_size: usize,
}

impl MatchDispatcher for DistributedDispatcher {
    fn dispatch_league(&self, matches: Vec<Match>) -> Result<Vec<MatchResult>, Vec<Match>> {
        let registry = self.registry.clone();
        let local_threads = self.local_threads;
        self.run_blocking(async move {
            let ready = registry.ready_handles().await;
            let slots = Self::build_slots(matches.len(), local_threads, ready);
            if slots.is_empty() {
                return Err(matches);
            }
            Ok(Self::execute_league(matches, slots, registry).await)
        })
    }

    fn dispatch_squads(
        &self,
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Result<Vec<(usize, MatchResultRaw)>, Vec<(usize, MatchSquad, MatchSquad, bool)>> {
        let registry = self.registry.clone();
        let local_threads = self.local_threads;
        self.run_blocking(async move {
            let ready = registry.ready_handles().await;
            let slots = Self::build_slots(matches.len(), local_threads, ready);
            if slots.is_empty() {
                return Err(matches);
            }
            Ok(Self::execute_squads(matches, slots, registry).await)
        })
    }
}

impl DistributedDispatcher {
    pub fn new(registry: WorkerRegistry, runtime: Handle, local_threads: usize) -> Self {
        DistributedDispatcher {
            registry,
            runtime,
            local_threads,
        }
    }

    /// Bridge sync→async without nested-runtime panics. The dispatcher
    /// is reached from sync code that is itself often running under an
    /// outer `Handle::block_on` (e.g. the simulator runs inside
    /// `spawn_blocking` + `handle.block_on(simulate())`). On that
    /// thread the runtime context is already active, so a second
    /// `block_on` panics with "Cannot start a runtime from within a
    /// runtime". `block_in_place` tells the multi-thread runtime to
    /// hand this thread's other work off to a sibling worker, after
    /// which we can drive a fresh `block_on` safely. When no runtime
    /// is current (e.g. a test calling `MatchPool::play` directly with
    /// no DistributedDispatcher installed but a hand-built one), fall
    /// back to plain `block_on`.
    fn run_blocking<F>(&self, fut: F) -> F::Output
    where
        F: Future,
    {
        if Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| self.runtime.block_on(fut))
        } else {
            self.runtime.block_on(fut)
        }
    }

    /// Build the dispatch targets for a batch: an optional local rayon
    /// slot plus one slot per Ready worker, each carrying the chunk size
    /// it pulls per round (`2 × threads`).
    ///
    /// Fast path: a batch that fits in a single local chunk skips every
    /// remote slot — the network/serialization round-trip would cost more
    /// than the local pool takes to play these matches. Cuts tail latency
    /// on idle days and the last stragglers of a matchday.
    ///
    /// Returns an empty vec when there is nothing to run on (no local
    /// share and no Ready workers); the caller then returns `Err` so the
    /// engine pool falls back to its own rayon path.
    fn build_slots(total: usize, local_threads: usize, ready: Vec<ReadyWorker>) -> Vec<Slot> {
        if total == 0 {
            return Vec::new();
        }
        if local_threads > 0 && total <= local_threads * 2 {
            return vec![Slot {
                target: Target::Local,
                chunk_size: local_threads * 2,
            }];
        }
        let mut slots = Vec::with_capacity(ready.len() + 1);
        if local_threads > 0 {
            slots.push(Slot {
                target: Target::Local,
                chunk_size: (local_threads * 2).max(1),
            });
        }
        for w in ready {
            let chunk_size = w.threads.max(1) * 2;
            slots.push(Slot {
                target: Target::Remote(w),
                chunk_size,
            });
        }
        slots
    }

    /// Drain up to `chunk_size` indices off the front of the shared
    /// pending queue. Returns an empty vec once the queue is drained —
    /// the slot loop reads that as "no more work, stop". Pure helper so
    /// the pull/clamp behaviour can be unit-tested without spawning
    /// TcpStream-bearing slots.
    fn pull_chunk(pending: &mut VecDeque<usize>, chunk_size: usize) -> Vec<usize> {
        let n = chunk_size.min(pending.len());
        pending.drain(0..n).collect()
    }

    fn describe_slots(slots: &[Slot]) -> String {
        slots
            .iter()
            .map(|s| format!("{}@{}", s.target.label(), s.chunk_size))
            .collect::<Vec<_>>()
            .join(", ")
    }

    async fn execute_league(
        matches: Vec<Match>,
        slots: Vec<Slot>,
        registry: WorkerRegistry,
    ) -> Vec<MatchResult> {
        let total = matches.len();
        let lg_label = matches
            .first()
            .map(|m| m.league_id().to_string())
            .unwrap_or_else(|| "?".to_string());
        info!(
            "dispatch league={} matches={} slots=[{}]",
            lg_label,
            total,
            Self::describe_slots(&slots)
        );

        let matches = Arc::new(matches);
        let pending = Arc::new(Mutex::new((0..total).collect::<VecDeque<usize>>()));

        let mut tasks = Vec::with_capacity(slots.len());
        for slot in slots {
            let matches = Arc::clone(&matches);
            let pending = Arc::clone(&pending);
            let registry = registry.clone();
            tasks.push(tokio::spawn(async move {
                Self::run_league_slot(slot, matches, pending, registry).await
            }));
        }

        let mut results: Vec<Option<MatchResult>> = (0..total).map(|_| None).collect();
        for t in tasks {
            if let Ok(items) = t.await {
                for (i, r) in items {
                    if i < results.len() {
                        results[i] = Some(r);
                    }
                }
            }
        }

        // Safety net: a match is still `None` only when every slot that
        // could have run it failed (or a slot task panicked outright).
        // Play the remainder on the local rayon pool so the caller always
        // gets a full result set — a total worker wipeout degrades to
        // local-only, it never drops matches.
        let missing: Vec<usize> = results
            .iter()
            .enumerate()
            .filter(|(_, r)| r.is_none())
            .map(|(i, _)| i)
            .collect();
        if !missing.is_empty() {
            warn!(
                "dispatch league={}: {} matches unprocessed after worker failures — running locally",
                lg_label,
                missing.len()
            );
            let chunk: Vec<Match> = missing.iter().map(|&i| matches[i].clone()).collect();
            let local =
                tokio::task::spawn_blocking(move || MatchRuntime::engine_pool().play_local(chunk))
                    .await
                    .unwrap_or_default();
            for (i, r) in missing.into_iter().zip(local) {
                results[i] = Some(r);
            }
        }

        results
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                r.unwrap_or_else(|| {
                    warn!(
                        "dispatcher: missing result for input index {} after safety net",
                        i
                    );
                    Self::placeholder_match_result()
                })
            })
            .collect()
    }

    async fn run_league_slot(
        slot: Slot,
        matches: Arc<Vec<Match>>,
        pending: Arc<Mutex<VecDeque<usize>>>,
        registry: WorkerRegistry,
    ) -> Vec<(usize, MatchResult)> {
        let mut out: Vec<(usize, MatchResult)> = Vec::new();
        loop {
            let indices = {
                let mut q = pending.lock().await;
                Self::pull_chunk(&mut q, slot.chunk_size)
            };
            if indices.is_empty() {
                break;
            }
            let count = indices.len();
            let chunk: Vec<Match> = indices.iter().map(|&i| matches[i].clone()).collect();
            match &slot.target {
                Target::Local => {
                    let timer = LatencyTimer::start();
                    let results = tokio::task::spawn_blocking(move || {
                        MatchRuntime::engine_pool().play_local(chunk)
                    })
                    .await
                    .unwrap_or_default();
                    let elapsed = timer.elapsed();
                    info!(
                        "local: completed league chunk matches={} in {} ms",
                        count,
                        elapsed.as_millis()
                    );
                    registry.record_local_batch(count, elapsed).await;
                    for (i, r) in indices.into_iter().zip(results) {
                        out.push((i, r));
                    }
                }
                Target::Remote(worker) => {
                    match Self::play_remote_league(worker, &registry, chunk).await {
                        Ok(results) => {
                            for (i, r) in indices.into_iter().zip(results) {
                                out.push((i, r));
                            }
                        }
                        Err(()) => {
                            // The worker died on this chunk. `record_batch`
                            // (inside play_remote_league) has already marked
                            // it Unreachable and dropped its connection, so
                            // it's gone from the next dispatch's ready set.
                            // Hand the chunk back to the front of the queue
                            // for a healthy slot to retry, then stop pulling
                            // — this slot must not touch the dead worker
                            // again.
                            Self::requeue_front(&pending, indices).await;
                            warn!(
                                "worker {} fenced off after failure; {} matches requeued",
                                worker.address, count
                            );
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    /// Send one request over a worker's connection and read its reply,
    /// bounded by [`REMOTE_BATCH_TIMEOUT`]. On timeout the partially
    /// driven write/read future is dropped; the caller then fences the
    /// worker and discards the connection, so the half-consumed stream is
    /// never reused.
    async fn request_with_timeout(stream: &mut TcpStream, req: &Request) -> io::Result<Response> {
        match tokio::time::timeout(REMOTE_BATCH_TIMEOUT, async {
            Frame::write(stream, req).await?;
            Frame::read(stream).await
        })
        .await
        {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "no response within {}s — worker fenced",
                    REMOTE_BATCH_TIMEOUT.as_secs()
                ),
            )),
        }
    }

    async fn play_remote_league(
        worker: &ReadyWorker,
        registry: &WorkerRegistry,
        matches: Vec<Match>,
    ) -> Result<Vec<MatchResult>, ()> {
        let count = matches.len();
        let envelopes: Vec<MatchEnvelope> = matches
            .iter()
            .map(|m| MatchEnvelope::League(LeagueMatchWire::from_match(m)))
            .collect();
        let req = Request::PlayBatch { items: envelopes };

        let mut stream = worker.connection.lock().await;
        let timer = LatencyTimer::start();
        let recv = Self::request_with_timeout(&mut stream, &req).await;
        drop(stream);
        let latency = timer.elapsed_ms();

        match recv {
            Ok(Response::PlayBatch { items }) if items.len() == count => {
                info!(
                    "remote {}: completed league chunk matches={} in {} ms",
                    worker.address, count, latency
                );
                registry
                    .record_batch(&worker.address, count, latency, BatchOutcome::Ok)
                    .await;
                Ok(items
                    .into_iter()
                    .filter_map(|o| match o {
                        MatchOutcome::League(r) => Some(r),
                        _ => None,
                    })
                    .collect())
            }
            other => {
                let reason = match other {
                    Ok(Response::Error { reason }) => reason,
                    Ok(_) => "unexpected response shape".to_string(),
                    Err(e) => format!("io: {}", e),
                };
                warn!(
                    "remote {}: league chunk failed — {}; requeueing",
                    worker.address, reason
                );
                registry
                    .record_batch(
                        &worker.address,
                        count,
                        latency,
                        BatchOutcome::Failed(reason),
                    )
                    .await;
                Err(())
            }
        }
    }

    async fn execute_squads(
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
        slots: Vec<Slot>,
        registry: WorkerRegistry,
    ) -> Vec<(usize, MatchResultRaw)> {
        let total = matches.len();
        info!(
            "dispatch squads matches={} slots=[{}]",
            total,
            Self::describe_slots(&slots)
        );

        let matches = Arc::new(matches);
        let pending = Arc::new(Mutex::new((0..total).collect::<VecDeque<usize>>()));

        let mut tasks = Vec::with_capacity(slots.len());
        for slot in slots {
            let matches = Arc::clone(&matches);
            let pending = Arc::clone(&pending);
            let registry = registry.clone();
            tasks.push(tokio::spawn(async move {
                Self::run_squad_slot(slot, matches, pending, registry).await
            }));
        }

        // Squad results carry the caller's fixture idx, not the input
        // position, so completeness is tracked by position separately.
        let mut done = vec![false; total];
        let mut out: Vec<(usize, MatchResultRaw)> = Vec::with_capacity(total);
        for t in tasks {
            if let Ok(items) = t.await {
                for (pos, pair) in items {
                    if pos < done.len() && !done[pos] {
                        done[pos] = true;
                        out.push(pair);
                    }
                }
            }
        }

        let missing: Vec<usize> = (0..total).filter(|&p| !done[p]).collect();
        if !missing.is_empty() {
            warn!(
                "dispatch squads: {} matches unprocessed after worker failures — running locally",
                missing.len()
            );
            let chunk: Vec<(usize, MatchSquad, MatchSquad, bool)> =
                missing.iter().map(|&p| matches[p].clone()).collect();
            let local = tokio::task::spawn_blocking(move || {
                MatchRuntime::engine_pool().play_squads_local(chunk)
            })
            .await
            .unwrap_or_default();
            out.extend(local);
        }

        out
    }

    async fn run_squad_slot(
        slot: Slot,
        matches: Arc<Vec<(usize, MatchSquad, MatchSquad, bool)>>,
        pending: Arc<Mutex<VecDeque<usize>>>,
        registry: WorkerRegistry,
    ) -> Vec<(usize, (usize, MatchResultRaw))> {
        let mut out: Vec<(usize, (usize, MatchResultRaw))> = Vec::new();
        loop {
            let indices = {
                let mut q = pending.lock().await;
                Self::pull_chunk(&mut q, slot.chunk_size)
            };
            if indices.is_empty() {
                break;
            }
            let count = indices.len();
            let chunk: Vec<(usize, MatchSquad, MatchSquad, bool)> =
                indices.iter().map(|&i| matches[i].clone()).collect();
            match &slot.target {
                Target::Local => {
                    let timer = LatencyTimer::start();
                    let results = tokio::task::spawn_blocking(move || {
                        MatchRuntime::engine_pool().play_squads_local(chunk)
                    })
                    .await
                    .unwrap_or_default();
                    let elapsed = timer.elapsed();
                    info!(
                        "local: completed squad chunk matches={} in {} ms",
                        count,
                        elapsed.as_millis()
                    );
                    registry.record_local_batch(count, elapsed).await;
                    for (pos, pair) in indices.into_iter().zip(results) {
                        out.push((pos, pair));
                    }
                }
                Target::Remote(worker) => {
                    match Self::play_remote_squads(worker, &registry, chunk).await {
                        Ok(results) => {
                            for (pos, pair) in indices.into_iter().zip(results) {
                                out.push((pos, pair));
                            }
                        }
                        Err(()) => {
                            Self::requeue_front(&pending, indices).await;
                            warn!(
                                "worker {} fenced off after failure; {} matches requeued",
                                worker.address, count
                            );
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    async fn play_remote_squads(
        worker: &ReadyWorker,
        registry: &WorkerRegistry,
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Result<Vec<(usize, MatchResultRaw)>, ()> {
        let count = matches.len();
        let envelopes: Vec<MatchEnvelope> = matches
            .iter()
            .map(|(idx, h, a, ko)| {
                MatchEnvelope::Squad(SquadFixtureWire {
                    idx: *idx,
                    is_knockout: *ko,
                    home: SquadWire::from_squad(h),
                    away: SquadWire::from_squad(a),
                })
            })
            .collect();
        let req = Request::PlayBatch { items: envelopes };

        let mut stream = worker.connection.lock().await;
        let timer = LatencyTimer::start();
        let recv = Self::request_with_timeout(&mut stream, &req).await;
        drop(stream);
        let latency = timer.elapsed_ms();

        match recv {
            Ok(Response::PlayBatch { items }) if items.len() == count => {
                info!(
                    "remote {}: completed squad chunk matches={} in {} ms",
                    worker.address, count, latency
                );
                registry
                    .record_batch(&worker.address, count, latency, BatchOutcome::Ok)
                    .await;
                Ok(items
                    .into_iter()
                    .filter_map(|o| match o {
                        MatchOutcome::Squad { idx, result } => Some((idx, result)),
                        _ => None,
                    })
                    .collect())
            }
            other => {
                let reason = match other {
                    Ok(Response::Error { reason }) => reason,
                    Ok(_) => "unexpected response shape".to_string(),
                    Err(e) => format!("io: {}", e),
                };
                warn!(
                    "remote {}: squad chunk failed — {}; requeueing",
                    worker.address, reason
                );
                registry
                    .record_batch(
                        &worker.address,
                        count,
                        latency,
                        BatchOutcome::Failed(reason),
                    )
                    .await;
                Err(())
            }
        }
    }

    /// Push a fenced-off worker's in-flight indices back onto the FRONT
    /// of the shared queue (order preserved) so a healthy slot retries
    /// them next, rather than after every still-queued chunk.
    async fn requeue_front(pending: &Arc<Mutex<VecDeque<usize>>>, indices: Vec<usize>) {
        let mut q = pending.lock().await;
        for i in indices.into_iter().rev() {
            q.push_front(i);
        }
    }

    fn placeholder_match_result() -> MatchResult {
        MatchResult {
            id: String::new(),
            league_id: 0,
            league_slug: String::new(),
            home_team_id: 0,
            away_team_id: 0,
            score: Score::new(0, 0),
            details: None,
            friendly: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DistributedDispatcher, Target};
    use std::collections::VecDeque;

    #[test]
    fn pull_chunk_drains_up_to_chunk_size() {
        let mut q: VecDeque<usize> = (0..10).collect();
        let chunk = DistributedDispatcher::pull_chunk(&mut q, 4);
        assert_eq!(chunk, vec![0, 1, 2, 3]);
        assert_eq!(q.len(), 6);
    }

    #[test]
    fn pull_chunk_clamps_to_remaining() {
        let mut q: VecDeque<usize> = (0..3).collect();
        let chunk = DistributedDispatcher::pull_chunk(&mut q, 8);
        assert_eq!(chunk, vec![0, 1, 2]);
        assert!(q.is_empty());
    }

    #[test]
    fn pull_chunk_empty_queue_returns_empty() {
        let mut q: VecDeque<usize> = VecDeque::new();
        let chunk = DistributedDispatcher::pull_chunk(&mut q, 8);
        assert!(chunk.is_empty());
    }

    /// Small batch with a local share collapses to a single local slot —
    /// no remote round-trip for work the local pool can swallow in one
    /// chunk.
    #[test]
    fn build_slots_fast_path_local_only() {
        let slots = DistributedDispatcher::build_slots(4, 8, Vec::new());
        assert_eq!(slots.len(), 1);
        assert!(matches!(slots[0].target, Target::Local));
        assert_eq!(slots[0].chunk_size, 16);
    }

    /// No local share and no Ready workers → no slots → caller returns
    /// `Err` and the engine pool runs its own rayon path.
    #[test]
    fn build_slots_no_targets_is_empty() {
        let slots = DistributedDispatcher::build_slots(100, 0, Vec::new());
        assert!(slots.is_empty());
    }

    /// Large batch, local enabled, no workers → one local slot that pulls
    /// the whole queue over multiple rounds.
    #[test]
    fn build_slots_local_only_large_batch() {
        let slots = DistributedDispatcher::build_slots(100, 8, Vec::new());
        assert_eq!(slots.len(), 1);
        assert!(matches!(slots[0].target, Target::Local));
    }
}
