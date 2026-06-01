//! Coordinator-side `MatchDispatcher` impl. Each dispatch_* call
//! SPLITS its batch into chunks of `2 × target.threads` and ships
//! chunks across every ready worker (plus the local rayon pool) in
//! parallel — so a single matchday doesn't sit on one worker while
//! the rest of the fleet idles.
//!
//! Routing model:
//!
//!   1. Snapshot the `Ready` worker slots, each carrying an EWMA
//!      throughput estimate the registry has been maintaining from
//!      observed batch latencies (`matches / latency_ms`). Workers
//!      that have never completed a batch read `None` and fall back
//!      to a thread-count seed.
//!   2. Prepend a virtual `Local` target (skipped when
//!      `local_threads = 0`) with its own throughput estimate.
//!   3. Smooth-weighted round-robin (SWRR) over the slots: each step
//!      every slot's `current_weight` is incremented by its weight,
//!      the slot with the highest `current_weight` wins the next
//!      chunk and its counter is reduced by `total_weight`. Chunks
//!      stay at `2 × threads` so the worker's rayon pool stays the
//!      right "size unit", but the FREQUENCY of chunk assignment is
//!      proportional to throughput — a 5× slower worker gets ~1/5 the
//!      chunks of a fast one with the same thread count, instead of
//!      becoming the matchday's tail latency. The per-call cursor
//!      rotates the slot order so concurrent dispatch calls (parallel
//!      `countries.par_iter_mut()`) don't all start at slot 0.
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
use std::future::Future;
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

/// Per-thread seed throughput (matches per second) used when a slot
/// has never recorded a batch. Picked so that an unmeasured slot's
/// initial weight is in the same order of magnitude as a measured
/// one — a real match takes a few tens of ms per thread, so 20 mps
/// per thread is a sensible neutral starting point that scales
/// linearly with thread count (matching the old behaviour exactly
/// when no measurements exist yet).
const SEED_MPS_PER_THREAD: f64 = 20.0;

impl MatchDispatcher for DistributedDispatcher {
    fn dispatch_league(&self, matches: Vec<Match>) -> Result<Vec<MatchResult>, Vec<Match>> {
        let registry = self.registry.clone();
        let local_threads = self.local_threads;
        let cursor_start = self.cursor.fetch_add(1, Ordering::Relaxed);
        self.run_blocking(async move {
            let ready = registry.ready_handles().await;
            let local_throughput = registry.local_throughput().await;
            let slots = Self::build_plan(
                matches.len(),
                local_threads,
                local_throughput,
                ready,
                cursor_start,
            );
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
        self.run_blocking(async move {
            let ready = registry.ready_handles().await;
            let local_throughput = registry.local_throughput().await;
            let slots = Self::build_plan(
                matches.len(),
                local_threads,
                local_throughput,
                ready,
                cursor_start,
            );
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

    /// Smooth-weighted round-robin chunking. Each step every slot's
    /// `current_weight` accumulates by its `weight` (matches/sec, from
    /// the registry's EWMA throughput or a thread-count seed when no
    /// batch has ever been observed); the slot with the highest
    /// `current_weight` claims the next chunk and gets `total_weight`
    /// subtracted from its counter. This produces a smooth
    /// interleaving where chunk frequency tracks throughput — a 5×
    /// slower worker gets ~1/5 the chunks of a fast one regardless of
    /// thread count.
    ///
    /// `chunk_size` stays at `2 × threads` per slot so each `PlayBatch`
    /// round-trip is still sized to fully occupy that worker's rayon
    /// pool without straggler tails on the chunk's own internal
    /// parallelism — slow workers just get FEWER chunks, not smaller
    /// ones.
    ///
    /// `cursor_start` is folded in by rotating the slot order before
    /// SWRR begins; without it, every dispatch call would tie-break
    /// toward the same slot on its first pick and concurrent dispatch
    /// calls would pile on the same worker.
    fn build_plan(
        total: usize,
        local_threads: usize,
        local_throughput_mpms: Option<f64>,
        ready: Vec<ReadyWorker>,
        cursor_start: usize,
    ) -> Vec<Slot> {
        // Fast path: the whole batch fits in one local chunk
        // (`2 × local_threads` is the local slot's `chunk_size`). Skip
        // every remote slot — the network/serialization round-trip
        // costs more than the local pool would take to play these
        // matches. Cuts tail latency on idle days, late-stage knockout
        // rounds, and the last few stragglers of any matchday.
        if local_threads > 0 && total > 0 && total <= local_threads * 2 {
            return vec![Slot {
                target: Target::Local,
                threads: local_threads,
                chunks: vec![(0..total).collect()],
            }];
        }
        let mut slots: Vec<Slot> = Vec::with_capacity(ready.len() + 1);
        let mut weights: Vec<f64> = Vec::with_capacity(ready.len() + 1);
        if local_threads > 0 {
            slots.push(Slot {
                target: Target::Local,
                threads: local_threads,
                chunks: Vec::new(),
            });
            weights.push(Self::slot_weight(local_throughput_mpms, local_threads));
        }
        for w in ready {
            let threads = w.threads;
            weights.push(Self::slot_weight(w.throughput_mpms, threads));
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
        let rotate = cursor_start % n;
        slots.rotate_left(rotate);
        weights.rotate_left(rotate);

        let chunk_sizes: Vec<usize> = slots.iter().map(|s| s.chunk_size()).collect();
        let assignments = Self::swrr_assign(total, &chunk_sizes, &weights);
        for (i, chunks) in assignments.into_iter().enumerate() {
            slots[i].chunks = chunks;
        }

        slots.into_iter().filter(|s| !s.chunks.is_empty()).collect()
    }

    /// Pure SWRR chunk-assignment loop. Returns per-slot lists of
    /// index ranges. Extracted from `build_plan` so the weighting
    /// behaviour can be unit-tested without TcpStream-bearing
    /// `ReadyWorker` instances.
    fn swrr_assign(
        total: usize,
        chunk_sizes: &[usize],
        weights: &[f64],
    ) -> Vec<Vec<Vec<usize>>> {
        let n = chunk_sizes.len();
        let mut out: Vec<Vec<Vec<usize>>> = (0..n).map(|_| Vec::new()).collect();
        if total == 0 || n == 0 {
            return out;
        }
        let total_weight: f64 = weights.iter().sum();
        let mut current: Vec<f64> = vec![0.0; n];
        let mut start = 0usize;
        while start < total {
            for i in 0..n {
                current[i] += weights[i];
            }
            let pick = current
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| {
                    a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
            let end = (start + chunk_sizes[pick]).min(total);
            out[pick].push((start..end).collect());
            current[pick] -= total_weight;
            start = end;
        }
        out
    }

    /// Weight (matches/sec) the slot will be assigned in SWRR. Real
    /// EWMA throughput when available, else a thread-count seed —
    /// `SEED_MPS_PER_THREAD × threads` matches the dispatcher's old
    /// behaviour exactly when nothing has been measured yet.
    fn slot_weight(throughput_mpms: Option<f64>, threads: usize) -> f64 {
        match throughput_mpms {
            Some(mpms) if mpms > 0.0 => (mpms * 1000.0).max(1.0),
            _ => (threads.max(1) as f64) * SEED_MPS_PER_THREAD,
        }
    }

    fn describe_plan(slots: &[Slot]) -> String {
        slots
            .iter()
            .map(|s| format!("{}×{} matches/{} chunks", s.target.label(), s.total(), s.chunks.len()))
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
                    let elapsed = timer.elapsed();
                    info!(
                        "local: completed league chunk matches={} in {} ms",
                        count,
                        elapsed.as_millis()
                    );
                    registry.record_local_batch(count, elapsed).await;
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
                    let elapsed = timer.elapsed();
                    info!(
                        "local: completed squad chunk matches={} in {} ms",
                        count,
                        elapsed.as_millis()
                    );
                    registry.record_local_batch(count, elapsed).await;
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

#[cfg(test)]
mod tests {
    use super::DistributedDispatcher;

    /// Two slots, same `chunk_size`, but slot B is 4× faster: B should
    /// receive roughly 4× as many chunks as A.
    #[test]
    fn swrr_assigns_chunks_proportional_to_weight() {
        let chunk_sizes = vec![8, 8];
        let weights = vec![100.0, 400.0]; // matches/sec — B is 4× A
        let assignments = DistributedDispatcher::swrr_assign(200, &chunk_sizes, &weights);
        let a = assignments[0].len();
        let b = assignments[1].len();
        assert!(a + b > 0);
        let ratio = b as f64 / a.max(1) as f64;
        assert!(
            ratio >= 3.5 && ratio <= 4.5,
            "expected B/A ≈ 4, got {:.2} ({} vs {})",
            ratio,
            b,
            a
        );
        let total_matches: usize = assignments
            .iter()
            .flat_map(|cs| cs.iter().map(|c| c.len()))
            .sum();
        assert_eq!(total_matches, 200);
    }

    /// Equal weights → pure round-robin, no slot starves.
    #[test]
    fn swrr_with_equal_weights_distributes_evenly() {
        let chunk_sizes = vec![8, 8, 8];
        let weights = vec![1.0, 1.0, 1.0];
        let assignments = DistributedDispatcher::swrr_assign(96, &chunk_sizes, &weights);
        for slot in &assignments {
            assert!(!slot.is_empty(), "every slot should get at least one chunk");
            let count: usize = slot.iter().map(|c| c.len()).sum();
            assert!(
                count >= 24 && count <= 40,
                "even split: each slot near 32, got {}",
                count
            );
        }
        let total: usize = assignments
            .iter()
            .flat_map(|cs| cs.iter().map(|c| c.len()))
            .sum();
        assert_eq!(total, 96);
    }

    /// A slot whose weight is dwarfed by the others should still get
    /// SOME work eventually — SWRR doesn't starve low-weight slots.
    #[test]
    fn swrr_does_not_starve_low_weight_slot() {
        let chunk_sizes = vec![16, 16];
        let weights = vec![1000.0, 10.0]; // 100× ratio
        let assignments = DistributedDispatcher::swrr_assign(500, &chunk_sizes, &weights);
        let fast: usize = assignments[0].iter().map(|c| c.len()).sum();
        let slow: usize = assignments[1].iter().map(|c| c.len()).sum();
        assert_eq!(fast + slow, 500);
        // Slow slot SHOULD get some chunks once enough total work
        // accumulates — every ~100 chunks for the fast slot, the slow
        // slot's current_weight overflows enough to pick.
        // With chunk_size=16 and total=500, fast claims ~30+ chunks
        // before slow gets one. Just assert slow > 0.
        // Allow strict starvation when ratio is extreme AND total fits
        // in fewer rounds than the ratio — here total/chunk_size ≈ 31
        // rounds, much less than 100, so slow may legitimately get 0.
        // Tighten with bigger total instead.
        let assignments = DistributedDispatcher::swrr_assign(5000, &chunk_sizes, &weights);
        let slow: usize = assignments[1].iter().map(|c| c.len()).sum();
        assert!(slow > 0, "slow slot should get at least one chunk over 5000 matches");
    }

    /// `slot_weight` falls back to thread-count × SEED when throughput
    /// is None, preserving today's behaviour for unmeasured slots.
    #[test]
    fn slot_weight_seed_for_unmeasured_slot() {
        let measured = DistributedDispatcher::slot_weight(Some(0.2), 16);
        // 0.2 mpms × 1000 = 200 mps
        assert!((measured - 200.0).abs() < 0.01);
        let seed = DistributedDispatcher::slot_weight(None, 16);
        // 16 × 20 = 320
        assert!((seed - 320.0).abs() < 0.01);
    }
}
