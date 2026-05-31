//! Coordinator-side `MatchDispatcher` impl. Each dispatch_* call
//! SPLITS its batch into chunks of `2 × target.threads` and ships
//! chunks to every ready worker (plus the local rayon pool) in
//! parallel — so a single matchday doesn't sit on one worker while
//! the rest of the fleet idles.
//!
//! Routing model:
//!
//!   1. Snapshot the `Ready` worker slots.
//!   2. Prepend a virtual `Local` target with `local_threads` weight
//!      (skipped when `local_threads = 0`).
//!   3. Greedy round-robin chunks of `2 × target.threads`: target
//!      `i` claims up to `2 × threads_i` consecutive matches, then
//!      the cursor advances. Small batches (≤ chunk cap) ship whole
//!      to ONE worker — splitting a 6-match batch 3/3 wastes the
//!      remote worker's rayon pool on network overhead and only
//!      partially fills its threads. The per-call cursor rotates the
//!      starting target so concurrent dispatch calls (parallel
//!      `countries.par_iter_mut()`) spread across the fleet.
//!   4. Spawn one tokio task per target slot. Chunks within a slot
//!      serialize over its `Mutex<TcpStream>`; slots run concurrently.
//!   5. Any failed remote chunk replays on the local rayon pool
//!      before returning, so callers never see a partial result.
//!
//! Local fallback: with no ready targets the dispatcher returns `Err`
//! so the engine pool runs the rayon path on the unmodified input.

use crate::worker::protocol::{MatchEnvelope, MatchOutcome, Request, Response};
use crate::worker::registry::{BatchOutcome, LatencyTimer, ReadyWorker, WorkerRegistry};
use crate::worker::transport::Frame;
use crate::worker::wire::{LeagueMatchWire, SquadFixtureWire, SquadWire};
use core::MatchRuntime;
use core::r#match::{Match, MatchDispatcher, MatchResult, MatchResultRaw, MatchSquad, Score};
use log::{info, warn};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::runtime::Handle;

pub struct DistributedDispatcher {
    registry: WorkerRegistry,
    runtime: Handle,
    /// `0` disables the coordinator's local share — every batch is
    /// routed to a remote worker (or the rayon-fallback path when no
    /// workers are Ready).
    local_threads: usize,
    /// Rotates the round-robin's starting target per call. Small
    /// batches (a few matches against many targets) would otherwise
    /// always land on slot 0 first; the rotation spreads those across
    /// concurrent dispatch calls.
    cursor: AtomicUsize,
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

/// One target's slice of a dispatched batch: the per-chunk lists of
/// input-vector indices to send. Chunks are sized at `2 × threads` so
/// each remote `PlayBatch` round-trip carries an amount of work the
/// worker's rayon pool can fully occupy without stragglers stalling a
/// huge batch.
struct Slot {
    target: Target,
    threads: usize,
    chunks: Vec<Vec<usize>>,
}

impl Slot {
    fn chunk_size(&self) -> usize {
        self.threads.max(1) * 2
    }

    fn total(&self) -> usize {
        self.chunks.iter().map(|c| c.len()).sum()
    }
}

impl MatchDispatcher for DistributedDispatcher {
    fn dispatch_league(&self, matches: Vec<Match>) -> Result<Vec<MatchResult>, Vec<Match>> {
        let registry = self.registry.clone();
        let local_threads = self.local_threads;
        let cursor_start = self.cursor.fetch_add(1, Ordering::Relaxed);
        self.runtime.block_on(async move {
            let ready = registry.ready_handles().await;
            let slots = Self::build_plan(matches.len(), local_threads, ready, cursor_start);
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
        let cursor_start = self.cursor.fetch_add(1, Ordering::Relaxed);
        self.runtime.block_on(async move {
            let ready = registry.ready_handles().await;
            let slots = Self::build_plan(matches.len(), local_threads, ready, cursor_start);
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
            cursor: AtomicUsize::new(0),
        }
    }

    /// Greedy round-robin chunking. Each round, the cursor's target
    /// claims up to `2 × threads` consecutive matches, then the
    /// cursor advances. Small batches (total ≤ chunk cap of the
    /// cursor's first pick) ship whole to ONE worker; that worker's
    /// rayon pool fills up rather than splitting a tiny batch across
    /// targets and paying network overhead per side. Large batches
    /// (total > one chunk cap) splay across the fleet, one chunk per
    /// target per round-trip.
    fn build_plan(
        total: usize,
        local_threads: usize,
        ready: Vec<ReadyWorker>,
        cursor_start: usize,
    ) -> Vec<Slot> {
        let mut slots: Vec<Slot> = Vec::with_capacity(ready.len() + 1);
        if local_threads > 0 {
            slots.push(Slot {
                target: Target::Local,
                threads: local_threads,
                chunks: Vec::new(),
            });
        }
        for w in ready {
            let threads = w.threads;
            slots.push(Slot {
                target: Target::Remote(w),
                threads,
                chunks: Vec::new(),
            });
        }
        if slots.is_empty() || total == 0 {
            return Vec::new();
        }

        let n = slots.len();
        let mut cursor = cursor_start % n;
        let mut start = 0usize;
        while start < total {
            let end = (start + slots[cursor].chunk_size()).min(total);
            slots[cursor].chunks.push((start..end).collect());
            start = end;
            cursor = (cursor + 1) % n;
        }

        slots.into_iter().filter(|s| !s.chunks.is_empty()).collect()
    }

    fn describe_plan(slots: &[Slot]) -> String {
        slots
            .iter()
            .map(|s| format!("{}×{}/{}c", s.target.label(), s.total(), s.chunks.len()))
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
            "dispatch league={} matches={} plan=[{}]",
            lg_label,
            total,
            Self::describe_plan(&slots)
        );

        let matches = Arc::new(matches);
        let mut tasks = Vec::with_capacity(slots.len());
        for slot in slots {
            let matches = Arc::clone(&matches);
            let registry = registry.clone();
            tasks.push(tokio::spawn(async move {
                Self::run_league_slot(slot, matches, registry).await
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

        // Defensive backfill — `play_remote_league` already replays
        // failed chunks locally before returning, so the only way to
        // reach here with a None is a tokio task that panicked outright.
        let mut originals: Vec<Option<Match>> = match Arc::try_unwrap(matches) {
            Ok(v) => v.into_iter().map(Some).collect(),
            Err(arc) => (*arc).iter().map(|m| Some(m.clone())).collect(),
        };
        let mut backfilled = 0usize;
        for (i, slot) in results.iter_mut().enumerate() {
            if slot.is_none() {
                if let Some(m) = originals[i].take() {
                    *slot = Some(m.play());
                    backfilled += 1;
                }
            }
        }
        if backfilled > 0 {
            warn!(
                "dispatch league={}: backfilled {} matches locally after slot task failure",
                lg_label, backfilled
            );
        }

        results
            .into_iter()
            .enumerate()
            .map(|(i, r)| {
                r.unwrap_or_else(|| {
                    warn!(
                        "dispatcher: missing result for input index {} after backfill",
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
        registry: WorkerRegistry,
    ) -> Vec<(usize, MatchResult)> {
        let mut out: Vec<(usize, MatchResult)> = Vec::with_capacity(slot.total());
        for indices in slot.chunks {
            let chunk: Vec<Match> = indices.iter().map(|&i| matches[i].clone()).collect();
            let count = chunk.len();
            let results = match &slot.target {
                Target::Local => {
                    let timer = LatencyTimer::start();
                    let r = tokio::task::spawn_blocking(move || {
                        MatchRuntime::engine_pool().play_local(chunk)
                    })
                    .await
                    .unwrap_or_default();
                    info!(
                        "local: completed league chunk matches={} in {} ms",
                        count,
                        timer.elapsed_ms()
                    );
                    r
                }
                Target::Remote(worker) => {
                    Self::play_remote_league(worker, &registry, chunk).await
                }
            };
            for (i, r) in indices.into_iter().zip(results) {
                out.push((i, r));
            }
        }
        out
    }

    async fn play_remote_league(
        worker: &ReadyWorker,
        registry: &WorkerRegistry,
        matches: Vec<Match>,
    ) -> Vec<MatchResult> {
        let count = matches.len();
        let envelopes: Vec<MatchEnvelope> = matches
            .iter()
            .map(|m| MatchEnvelope::League(LeagueMatchWire::from_match(m)))
            .collect();
        let req = Request::PlayBatch { items: envelopes };

        let mut stream = worker.connection.lock().await;
        let timer = LatencyTimer::start();
        let recv: std::io::Result<Response> = match Frame::write(&mut *stream, &req).await {
            Ok(()) => Frame::read(&mut *stream).await,
            Err(e) => Err(e),
        };
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
                items
                    .into_iter()
                    .filter_map(|o| match o {
                        MatchOutcome::League(r) => Some(r),
                        _ => None,
                    })
                    .collect()
            }
            other => {
                let reason = match other {
                    Ok(Response::Error { reason }) => reason,
                    Ok(_) => "unexpected response shape".to_string(),
                    Err(e) => format!("io: {}", e),
                };
                warn!(
                    "remote {}: league chunk failed — {}; running locally",
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
                tokio::task::spawn_blocking(move || MatchRuntime::engine_pool().play_local(matches))
                    .await
                    .unwrap_or_default()
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
            "dispatch squads matches={} plan=[{}]",
            total,
            Self::describe_plan(&slots)
        );

        let matches = Arc::new(matches);
        let mut tasks = Vec::with_capacity(slots.len());
        for slot in slots {
            let matches = Arc::clone(&matches);
            let registry = registry.clone();
            tasks.push(tokio::spawn(async move {
                Self::run_squad_slot(slot, matches, registry).await
            }));
        }

        let mut out: Vec<(usize, MatchResultRaw)> = Vec::with_capacity(total);
        for t in tasks {
            if let Ok(items) = t.await {
                out.extend(items);
            }
        }
        out
    }

    async fn run_squad_slot(
        slot: Slot,
        matches: Arc<Vec<(usize, MatchSquad, MatchSquad, bool)>>,
        registry: WorkerRegistry,
    ) -> Vec<(usize, MatchResultRaw)> {
        let mut out: Vec<(usize, MatchResultRaw)> = Vec::with_capacity(slot.total());
        for indices in slot.chunks {
            let chunk: Vec<(usize, MatchSquad, MatchSquad, bool)> =
                indices.iter().map(|&i| matches[i].clone()).collect();
            let count = chunk.len();
            let part = match &slot.target {
                Target::Local => {
                    let timer = LatencyTimer::start();
                    let r = tokio::task::spawn_blocking(move || {
                        MatchRuntime::engine_pool().play_squads_local(chunk)
                    })
                    .await
                    .unwrap_or_default();
                    info!(
                        "local: completed squad chunk matches={} in {} ms",
                        count,
                        timer.elapsed_ms()
                    );
                    r
                }
                Target::Remote(worker) => {
                    Self::play_remote_squads(worker, &registry, chunk).await
                }
            };
            out.extend(part);
        }
        out
    }

    async fn play_remote_squads(
        worker: &ReadyWorker,
        registry: &WorkerRegistry,
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Vec<(usize, MatchResultRaw)> {
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
        let recv: std::io::Result<Response> = match Frame::write(&mut *stream, &req).await {
            Ok(()) => Frame::read(&mut *stream).await,
            Err(e) => Err(e),
        };
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
                items
                    .into_iter()
                    .filter_map(|o| match o {
                        MatchOutcome::Squad { idx, result } => Some((idx, result)),
                        _ => None,
                    })
                    .collect()
            }
            other => {
                let reason = match other {
                    Ok(Response::Error { reason }) => reason,
                    Ok(_) => "unexpected response shape".to_string(),
                    Err(e) => format!("io: {}", e),
                };
                warn!(
                    "remote {}: squad chunk failed — {}; running locally",
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
                tokio::task::spawn_blocking(move || {
                    MatchRuntime::engine_pool().play_squads_local(matches)
                })
                .await
                .unwrap_or_default()
            }
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
