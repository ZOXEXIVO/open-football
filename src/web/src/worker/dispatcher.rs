//! Coordinator-side `MatchDispatcher` impl. Routes match batches over
//! the per-worker framed TCP connection set up by `WorkerRegistry`,
//! while keeping the coordinator's own CPU in the rotation as a
//! virtual "local" worker so the host doesn't sit idle while remote
//! workers crunch.
//!
//! Routing model:
//!
//!   1. Snapshot the `Ready` worker slots.
//!   2. Prepend a virtual `Local` target with `local_threads` weight
//!      (defaults to the coordinator's match-engine thread count).
//!      Skipped when `local_threads = 0`.
//!   3. Group league fixtures by `league_id` (single-league batches
//!      are the common case — those are striped per-match across the
//!      targets so all slots stay busy).
//!   4. Weighted round-robin by thread count decides which target gets
//!      which batch.
//!   5. One task per remote slot drains its assigned work over its own
//!      framed TCP connection. The local share runs on the local
//!      rayon pool concurrently.
//!   6. Any failed remote batch is re-played on the local pool before
//!      returning, so callers never see a partial result.

use crate::worker::protocol::{MatchEnvelope, MatchOutcome, Request, Response};
use crate::worker::registry::{BatchOutcome, LatencyTimer, ReadyWorker, WorkerRegistry};
use crate::worker::transport::Frame;
use crate::worker::wire::{LeagueMatchWire, SquadFixtureWire, SquadWire};
use core::MatchRuntime;
use core::r#match::{
    FootballEngine, Match, MatchDispatcher, MatchResult, MatchResultRaw, MatchSquad, Score,
};
use log::{info, warn};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::runtime::Handle;

pub struct DistributedDispatcher {
    registry: WorkerRegistry,
    runtime: Handle,
    /// Number of threads to weight the local coordinator's share by.
    /// `0` disables local processing entirely — every batch goes to a
    /// remote worker (or the rayon-fallback path when no workers are
    /// Ready).
    local_threads: usize,
}

impl DistributedDispatcher {
    pub fn new(registry: WorkerRegistry, runtime: Handle, local_threads: usize) -> Self {
        DistributedDispatcher {
            registry,
            runtime,
            local_threads,
        }
    }
}

/// A single routing target — either the coordinator's local rayon
/// pool or one ready remote slot. Built fresh per dispatch call so
/// the `Vec` is always non-empty when `dispatch_*` returns `Ok`.
#[derive(Clone)]
enum Target {
    Local { threads: usize },
    Remote(ReadyWorker),
}

impl Target {
    fn threads(&self) -> usize {
        match self {
            Target::Local { threads } => *threads,
            Target::Remote(w) => w.threads.max(1),
        }
    }

    fn label(&self) -> String {
        match self {
            Target::Local { threads } => format!("local ({} threads)", threads),
            Target::Remote(w) => w.address.clone(),
        }
    }
}

impl MatchDispatcher for DistributedDispatcher {
    fn dispatch_league(&self, matches: Vec<Match>) -> Result<Vec<MatchResult>, Vec<Match>> {
        let targets = self.runtime.block_on(self.build_targets());
        if targets.is_empty() {
            return Err(matches);
        }
        let plan = Self::plan_league(&matches, &targets);
        if plan.is_empty() {
            return Err(matches);
        }
        let registry = self.registry.clone();
        let outcomes = self.runtime.block_on(async move {
            Self::execute_league(plan, matches, targets, registry).await
        });
        Ok(outcomes)
    }

    fn dispatch_squads(
        &self,
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Result<Vec<(usize, MatchResultRaw)>, Vec<(usize, MatchSquad, MatchSquad, bool)>> {
        let targets = self.runtime.block_on(self.build_targets());
        if targets.is_empty() {
            return Err(matches);
        }
        let registry = self.registry.clone();
        let outcomes = self.runtime.block_on(async move {
            Self::execute_squads(matches, targets, registry).await
        });
        Ok(outcomes)
    }
}

/// Per-target assignment: list of input-vector indices that go to that
/// target. Index `0` is always the Local target when `local_threads >
/// 0`; remote slots follow in `ready_handles()` order.
type LeaguePlan = Vec<Vec<usize>>;

impl DistributedDispatcher {
    async fn build_targets(&self) -> Vec<Target> {
        let ready = self.registry.ready_handles().await;
        let mut targets: Vec<Target> = Vec::with_capacity(ready.len() + 1);
        if self.local_threads > 0 {
            targets.push(Target::Local {
                threads: self.local_threads,
            });
        }
        for w in ready {
            targets.push(Target::Remote(w));
        }
        targets
    }

    /// Build a per-target assignment for a batch of league fixtures.
    /// Single-league batches (the common case — one matchday in one
    /// league per call) get matches interleaved across targets by
    /// `weighted_round_robin` so all CPUs stay busy. Multi-league
    /// batches keep each league together on one target (locality of
    /// related post-match aggregation), with the league→target
    /// assignment itself produced by `weighted_round_robin`.
    fn plan_league(matches: &[Match], targets: &[Target]) -> LeaguePlan {
        if targets.is_empty() {
            return Vec::new();
        }
        let mut groups: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
        for (i, m) in matches.iter().enumerate() {
            groups.entry(m.league_id()).or_default().push(i);
        }
        let mut plan: LeaguePlan = vec![Vec::new(); targets.len()];
        if groups.len() == 1 {
            let (_lg, indices) = groups.into_iter().next().expect("one group");
            let assignment = Self::weighted_round_robin(indices.len(), targets);
            for (k, i) in indices.into_iter().enumerate() {
                plan[assignment[k]].push(i);
            }
        } else {
            let assignment = Self::weighted_round_robin(groups.len(), targets);
            for (k, (_lg, indices)) in groups.into_iter().enumerate() {
                plan[assignment[k]].extend(indices);
            }
        }
        plan
    }

    async fn execute_league(
        plan: LeaguePlan,
        matches: Vec<Match>,
        targets: Vec<Target>,
        registry: WorkerRegistry,
    ) -> Vec<MatchResult> {
        let total = matches.len();
        // League id of the batch — used only for the dispatch log line
        // when every input shares a league (the common case).
        let lg_label = matches
            .first()
            .map(|m| m.league_id().to_string())
            .unwrap_or_else(|| "?".to_string());
        let all_same_league = matches.iter().all(|m| m.league_id().to_string() == lg_label);
        if all_same_league {
            info!(
                "dispatch league={} matches={} targets={}",
                lg_label,
                total,
                Self::describe_plan(&plan, &targets)
            );
        } else {
            info!(
                "dispatch matches={} multi-league targets={}",
                total,
                Self::describe_plan(&plan, &targets)
            );
        }

        let matches = Arc::new(matches);
        let mut results: Vec<Option<MatchResult>> = (0..total).map(|_| None).collect();

        let mut tasks = Vec::with_capacity(targets.len());
        for (t_idx, indices) in plan.into_iter().enumerate() {
            if indices.is_empty() {
                continue;
            }
            let target = targets[t_idx].clone();
            let matches_ref = Arc::clone(&matches);
            let registry = registry.clone();
            tasks.push(tokio::spawn(async move {
                match target {
                    Target::Local { .. } => {
                        Self::run_local_league_batch(indices, matches_ref).await
                    }
                    Target::Remote(worker) => {
                        let payload: Vec<(usize, MatchEnvelope)> = indices
                            .iter()
                            .map(|&i| {
                                let env = MatchEnvelope::League(LeagueMatchWire::from_match(
                                    &matches_ref[i],
                                ));
                                (i, env)
                            })
                            .collect();
                        Self::run_remote_league_batch(worker, registry, payload).await
                    }
                }
            }));
        }

        for t in tasks {
            if let Ok(items) = t.await {
                for (idx, outcome) in items {
                    if let MatchOutcome::League(result) = outcome {
                        if idx < results.len() {
                            results[idx] = Some(result);
                        }
                    }
                }
            }
        }

        let mut originals: Vec<Option<Match>> = match Arc::try_unwrap(matches) {
            Ok(v) => v.into_iter().map(Some).collect(),
            Err(arc) => (*arc).iter().map(|m| Some(m.clone())).collect(),
        };

        let mut missing = 0usize;
        for (i, slot) in results.iter_mut().enumerate() {
            if slot.is_none() {
                if let Some(m) = originals[i].take() {
                    *slot = Some(m.play());
                    missing += 1;
                }
            }
        }
        if missing > 0 {
            info!(
                "dispatch league={}: backfilled {} matches locally after remote failure",
                lg_label, missing
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
                    placeholder_match_result()
                })
            })
            .collect()
    }

    async fn run_local_league_batch(
        indices: Vec<usize>,
        matches: Arc<Vec<Match>>,
    ) -> Vec<(usize, MatchOutcome)> {
        let count = indices.len();
        let timer = LatencyTimer::start();
        // Clone the specific subset for the blocking task so the Arc
        // stays available for any sibling task that may still be
        // borrowing. After all per-target tasks return, the outer
        // `Arc::try_unwrap` reclaims the originals.
        let subset: Vec<(usize, Match)> = indices
            .iter()
            .map(|&i| (i, matches[i].clone()))
            .collect();
        let result = tokio::task::spawn_blocking(move || {
            let played: Vec<(usize, MatchResult)> = MatchRuntime::engine_pool()
                .play_local(subset.iter().map(|(_, m)| m.clone()).collect())
                .into_iter()
                .zip(subset.iter().map(|(i, _)| *i))
                .map(|(r, i)| (i, r))
                .collect();
            played
        })
        .await
        .unwrap_or_default();
        info!(
            "local: completed league batch matches={} in {} ms",
            count,
            timer.elapsed_ms()
        );
        result
            .into_iter()
            .map(|(i, r)| (i, MatchOutcome::League(r)))
            .collect()
    }

    async fn run_remote_league_batch(
        worker: ReadyWorker,
        registry: WorkerRegistry,
        payload: Vec<(usize, MatchEnvelope)>,
    ) -> Vec<(usize, MatchOutcome)> {
        let count = payload.len();
        let (indices, envelopes): (Vec<usize>, Vec<MatchEnvelope>) = payload.into_iter().unzip();
        let req = Request::PlayBatch { items: envelopes };

        info!(
            "remote {}: sending league batch matches={}",
            worker.address, count
        );
        let mut stream = worker.connection.lock().await;
        let timer = LatencyTimer::start();
        if let Err(e) = Frame::write(&mut *stream, &req).await {
            let reason = format!("send: {}", e);
            warn!("remote {}: send failed — {}", worker.address, reason);
            registry
                .record_batch(
                    &worker.address,
                    count,
                    timer.elapsed_ms(),
                    BatchOutcome::Failed(reason),
                )
                .await;
            return Vec::new();
        }
        let resp: Response = match Frame::read(&mut *stream).await {
            Ok(r) => r,
            Err(e) => {
                let reason = format!("recv: {}", e);
                warn!("remote {}: recv failed — {}", worker.address, reason);
                registry
                    .record_batch(
                        &worker.address,
                        count,
                        timer.elapsed_ms(),
                        BatchOutcome::Failed(reason),
                    )
                    .await;
                return Vec::new();
            }
        };
        drop(stream);
        let latency = timer.elapsed_ms();

        match resp {
            Response::PlayBatch { items } if items.len() == indices.len() => {
                info!(
                    "remote {}: completed league batch matches={} in {} ms",
                    worker.address, count, latency
                );
                registry
                    .record_batch(&worker.address, count, latency, BatchOutcome::Ok)
                    .await;
                indices.into_iter().zip(items).collect()
            }
            Response::PlayBatch { items } => {
                let reason = format!(
                    "result count mismatch (sent {}, got {})",
                    indices.len(),
                    items.len()
                );
                warn!(
                    "remote {}: {} — falling back to local",
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
                Vec::new()
            }
            Response::Error { reason } => {
                warn!("remote {}: error response — {}", worker.address, reason);
                registry
                    .record_batch(
                        &worker.address,
                        count,
                        latency,
                        BatchOutcome::Failed(reason),
                    )
                    .await;
                Vec::new()
            }
            _ => {
                registry
                    .record_batch(
                        &worker.address,
                        count,
                        latency,
                        BatchOutcome::Failed("unexpected response".to_string()),
                    )
                    .await;
                Vec::new()
            }
        }
    }

    async fn execute_squads(
        matches: Vec<(usize, MatchSquad, MatchSquad, bool)>,
        targets: Vec<Target>,
        registry: WorkerRegistry,
    ) -> Vec<(usize, MatchResultRaw)> {
        let total = matches.len();
        info!(
            "dispatch squads matches={} targets={}",
            total,
            targets
                .iter()
                .map(|t| t.label())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let assignment = Self::weighted_round_robin(matches.len(), &targets);
        if assignment.is_empty() {
            return matches
                .into_iter()
                .map(|(idx, home, away, ko)| {
                    let r = FootballEngine::<840, 545>::play(home, away, false, false, ko);
                    (idx, r)
                })
                .collect();
        }

        // Bucket the input by target.
        let mut per_target: Vec<Vec<(usize, MatchSquad, MatchSquad, bool)>> =
            (0..targets.len()).map(|_| Vec::new()).collect();
        for (slot, (idx, home, away, ko)) in matches.into_iter().enumerate() {
            per_target[assignment[slot]].push((idx, home, away, ko));
        }

        let mut tasks = Vec::with_capacity(targets.len());
        for (t_idx, bucket) in per_target.into_iter().enumerate() {
            if bucket.is_empty() {
                continue;
            }
            let target = targets[t_idx].clone();
            let registry = registry.clone();
            tasks.push(tokio::spawn(async move {
                match target {
                    Target::Local { .. } => Self::run_local_squad_batch(bucket).await,
                    Target::Remote(worker) => {
                        Self::run_remote_squad_batch(worker, registry, bucket).await
                    }
                }
            }));
        }
        let mut out: Vec<(usize, MatchResultRaw)> = Vec::new();
        for t in tasks {
            if let Ok(items) = t.await {
                out.extend(items);
            }
        }
        out
    }

    async fn run_local_squad_batch(
        bucket: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Vec<(usize, MatchResultRaw)> {
        let count = bucket.len();
        let timer = LatencyTimer::start();
        let result = tokio::task::spawn_blocking(move || {
            MatchRuntime::engine_pool().play_squads_local(bucket)
        })
        .await
        .unwrap_or_default();
        info!(
            "local: completed squad batch matches={} in {} ms",
            count,
            timer.elapsed_ms()
        );
        result
    }

    async fn run_remote_squad_batch(
        worker: ReadyWorker,
        registry: WorkerRegistry,
        bucket: Vec<(usize, MatchSquad, MatchSquad, bool)>,
    ) -> Vec<(usize, MatchResultRaw)> {
        let count = bucket.len();
        let wires: Vec<SquadFixtureWire> = bucket
            .iter()
            .map(|(idx, home, away, ko)| SquadFixtureWire {
                idx: *idx,
                is_knockout: *ko,
                home: SquadWire::from_squad(home),
                away: SquadWire::from_squad(away),
            })
            .collect();
        let envelopes: Vec<MatchEnvelope> = wires.into_iter().map(MatchEnvelope::Squad).collect();
        let req = Request::PlayBatch { items: envelopes };

        info!(
            "remote {}: sending squad batch matches={}",
            worker.address, count
        );
        let mut stream = worker.connection.lock().await;
        let timer = LatencyTimer::start();
        let send = Frame::write(&mut *stream, &req).await;
        let recv: std::io::Result<Response> = match send {
            Ok(()) => Frame::read(&mut *stream).await,
            Err(e) => Err(e),
        };
        drop(stream);
        let latency = timer.elapsed_ms();

        match recv {
            Ok(Response::PlayBatch { items }) if items.len() == count => {
                info!(
                    "remote {}: completed squad batch matches={} in {} ms",
                    worker.address, count, latency
                );
                registry
                    .record_batch(&worker.address, count, latency, BatchOutcome::Ok)
                    .await;
                items
                    .into_iter()
                    .filter_map(|outcome| match outcome {
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
                    "remote {}: squad batch failed — {}; falling back to local",
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
                    bucket
                        .into_iter()
                        .map(|(idx, home, away, ko)| {
                            let r =
                                FootballEngine::<840, 545>::play(home, away, false, false, ko);
                            (idx, r)
                        })
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default()
            }
        }
    }

    /// Build a slot→target assignment of length `n` using weighted
    /// round-robin that INTERLEAVES targets (not block-distributes
    /// them). For thread counts [4,4] this produces [0,1,0,1,…] — 10
    /// matches give 5+5. Uneven thread counts (e.g. [8,4,2]) produce
    /// a fair interleaving where each target gets shares
    /// proportional to its `threads()`.
    fn weighted_round_robin(n: usize, targets: &[Target]) -> Vec<usize> {
        if targets.is_empty() || n == 0 {
            return Vec::new();
        }
        let mut counts = vec![0usize; targets.len()];
        let mut plan = Vec::with_capacity(n);
        for _ in 0..n {
            let pick = (0..targets.len())
                .min_by(|&a, &b| {
                    let ta = targets[a].threads().max(1);
                    let tb = targets[b].threads().max(1);
                    let la = counts[a] * tb;
                    let lb = counts[b] * ta;
                    la.cmp(&lb).then(a.cmp(&b))
                })
                .expect("at least one target");
            plan.push(pick);
            counts[pick] += 1;
        }
        plan
    }

    fn describe_plan(plan: &LeaguePlan, targets: &[Target]) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (i, indices) in plan.iter().enumerate() {
            if indices.is_empty() {
                continue;
            }
            parts.push(format!("{}×{}", targets[i].label(), indices.len()));
        }
        parts.join(", ")
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

impl Clone for ReadyWorker {
    fn clone(&self) -> Self {
        ReadyWorker {
            address: self.address.clone(),
            threads: self.threads,
            connection: Arc::clone(&self.connection),
        }
    }
}
