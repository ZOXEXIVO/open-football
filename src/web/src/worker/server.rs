//! Worker-mode TCP listener. When the binary is started with `--worker`,
//! `main` skips DB load and instead drives `WorkerServer::run` — a
//! plain `TcpListener` that handles one persistent connection per
//! coordinator. Each connection runs through one `Handshake` exchange
//! and then a stream of `PlayBatch` requests.

use crate::common::default_handler::{COMPUTER_NAME, CPU_BRAND};
use crate::worker::protocol::{MatchEnvelope, MatchOutcome, PROTOCOL_VERSION, Request, Response};
use crate::worker::registry::LatencyTimer;
use crate::worker::transport::Frame;
use core::MatchRuntime;
use core::r#match::{Match, MatchResult, MatchSquad, Score};
use log::{debug, error, info, warn};
use std::collections::HashMap;
use tokio::net::{TcpListener, TcpStream};

pub struct WorkerServer {
    port: u16,
}

impl WorkerServer {
    pub fn new(port: u16) -> Self {
        WorkerServer { port }
    }

    /// Bind on `0.0.0.0:port` and serve connections forever. Returns
    /// only on a fatal listener error.
    pub async fn run(self) {
        // Recording mode generates position tracks that the wire layer
        // drops (`MatchResultRaw.position_data` is `#[serde(skip)]`).
        // Recording is a coordinator-side feature for the local replay
        // viewer — a worker generating it just burns CPU and RAM that
        // never reach the wire. Force it off here, regardless of the
        // env var / CLI flag the worker process was started with.
        MatchRuntime::set_recordings_mode(false);
        MatchRuntime::set_events_mode(false);

        let addr = format!("0.0.0.0:{}", self.port);
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("worker: failed to bind {}: {}", addr, e);
                return;
            }
        };
        info!(
            "worker: listening on {} ({} match threads, v{}, protocol v{})",
            addr,
            MatchRuntime::engine_pool().num_threads(),
            env!("CARGO_PKG_VERSION"),
            PROTOCOL_VERSION,
        );

        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    debug!("worker: accepted connection from {}", peer);
                    tokio::spawn(async move {
                        if let Err(e) = WorkerConnection::new(stream).serve().await {
                            warn!("worker: connection from {} closed: {}", peer, e);
                        }
                    });
                }
                Err(e) => {
                    error!("worker: accept error: {}", e);
                    // Don't tight-loop on a persistent accept failure;
                    // give the listener a moment to recover.
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            }
        }
    }
}

struct WorkerConnection {
    stream: TcpStream,
}

impl WorkerConnection {
    fn new(stream: TcpStream) -> Self {
        WorkerConnection { stream }
    }

    async fn serve(mut self) -> std::io::Result<()> {
        // Handshake is the mandatory first frame on every connection.
        let first: Request = Frame::read(&mut self.stream).await?;
        match first {
            Request::Handshake {
                coordinator_version,
                protocol_version,
            } => {
                let our_version = env!("CARGO_PKG_VERSION");
                let version_ok = coordinator_version == our_version;
                let protocol_ok = protocol_version == PROTOCOL_VERSION;

                if !version_ok || !protocol_ok {
                    let reason = if !version_ok {
                        format!(
                            "version mismatch: coordinator {} worker {}",
                            coordinator_version, our_version
                        )
                    } else {
                        format!(
                            "protocol mismatch: coordinator {} worker {}",
                            protocol_version, PROTOCOL_VERSION
                        )
                    };
                    warn!("worker: rejecting handshake — {}", reason);
                    Frame::write(&mut self.stream, &Response::HandshakeRejected { reason }).await?;
                    return Ok(());
                }

                let reply = Response::Handshake {
                    version: our_version.to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    threads: MatchRuntime::engine_pool().num_threads(),
                    computer_name: COMPUTER_NAME.clone(),
                    cpu_brand: CPU_BRAND.clone(),
                };
                Frame::write(&mut self.stream, &reply).await?;
            }
            other => {
                let reason = format!(
                    "expected Handshake as first message, got {:?}",
                    std::mem::discriminant(&other)
                );
                warn!("worker: protocol violation — {}", reason);
                Frame::write(&mut self.stream, &Response::HandshakeRejected { reason }).await?;
                return Ok(());
            }
        }

        // Service loop — one Request → one Response, in order, until
        // the coordinator drops the connection.
        loop {
            let req: Request = match Frame::read(&mut self.stream).await {
                Ok(r) => r,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
                Err(e) => return Err(e),
            };
            match req {
                Request::PlayBatch { items } => {
                    // `play_batch` is CPU-bound (it runs the full match
                    // engine for every item). Run it on the blocking
                    // pool so this connection's tokio worker thread
                    // stays free to accept new connections and service
                    // heartbeat tickers — otherwise a long matchday
                    // would freeze every other concurrent batch and
                    // the listener's accept loop.
                    // `items` moves into the closure, so grab the count
                    // first and time just the processing (not the response
                    // write) for the completion log below.
                    let count = items.len();
                    let timer = LatencyTimer::start();
                    let outcomes =
                        match tokio::task::spawn_blocking(move || Self::play_batch(items)).await {
                            Ok(o) => o,
                            Err(e) => {
                                let resp = Response::Error {
                                    reason: format!("worker task panicked: {}", e),
                                };
                                Frame::write(&mut self.stream, &resp).await?;
                                continue;
                            }
                        };
                    info!(
                        "worker: processed batch matches={} in {} ms",
                        count,
                        timer.elapsed_ms()
                    );
                    let resp = Response::PlayBatch { items: outcomes };
                    Frame::write(&mut self.stream, &resp).await?;
                }
                Request::Ping => {
                    // Liveness probe — answer immediately. The worker's
                    // serve loop is single-threaded per connection, so a
                    // prompt pong proves both the socket and this task are
                    // healthy and not wedged.
                    Frame::write(&mut self.stream, &Response::Pong).await?;
                }
                Request::Handshake { .. } => {
                    // Already handshaked. A second handshake is a
                    // protocol violation; reply with an error and
                    // continue serving so the coordinator's side can
                    // log it.
                    let resp = Response::Error {
                        reason: "duplicate handshake".to_string(),
                    };
                    Frame::write(&mut self.stream, &resp).await?;
                }
            }
        }
    }

    /// Run a batch through the worker's local engine pool, in
    /// parallel. The previous version mapped serially over `into_iter`
    /// — a 10-match batch with `--match-threads 8` would still run
    /// matches one at a time, leaving 7 threads idle. This version
    /// dispatches league and squad envelopes through the pool's bulk
    /// APIs (`play` / `play_squads_with_knockout`), which rayon-fan
    /// across every configured match thread. The worker process has
    /// no `MatchDispatcher` installed, so those calls always take the
    /// local rayon path.
    fn play_batch(items: Vec<MatchEnvelope>) -> Vec<MatchOutcome> {
        let mut outcomes: Vec<Option<MatchOutcome>> = (0..items.len()).map(|_| None).collect();

        // Split envelopes by variant, remembering the original input
        // position so the response can be scattered back in order.
        let mut league: Vec<(usize, Match)> = Vec::new();
        let mut squad: Vec<(usize, usize, MatchSquad, MatchSquad, bool)> = Vec::new();
        for (input_pos, env) in items.into_iter().enumerate() {
            match env {
                MatchEnvelope::League(wire) => league.push((input_pos, wire.into_match())),
                MatchEnvelope::Squad(wire) => {
                    let caller_idx = wire.idx;
                    let home = wire.home.into_squad();
                    let away = wire.away.into_squad();
                    squad.push((input_pos, caller_idx, home, away, wire.is_knockout));
                }
            }
        }

        let pool = MatchRuntime::engine_pool();

        if !league.is_empty() {
            let (positions, matches): (Vec<usize>, Vec<_>) = league.into_iter().unzip();
            let results = pool.play(matches);
            for (pos, r) in positions.into_iter().zip(results) {
                outcomes[pos] = Some(MatchOutcome::League(r));
            }
        }
        if !squad.is_empty() {
            // Use the input position as the synthetic idx into
            // `play_squads_with_knockout`, since that idx is only used
            // by the pool itself to pair input with output. We carry
            // the caller_idx (the wire's idx — the coordinator's
            // original fixture id) separately and stamp it on the
            // outgoing `MatchOutcome::Squad { idx }` so the
            // coordinator can find its fixture again.
            let mut keyed = Vec::with_capacity(squad.len());
            let mut caller_idx_by_pos: HashMap<usize, usize> = HashMap::with_capacity(squad.len());
            for (pos, caller_idx, home, away, ko) in squad {
                caller_idx_by_pos.insert(pos, caller_idx);
                keyed.push((pos, home, away, ko));
            }
            let results = pool.play_squads_with_knockout(keyed);
            for (pos, raw) in results {
                let caller_idx = *caller_idx_by_pos.get(&pos).unwrap_or(&pos);
                outcomes[pos] = Some(MatchOutcome::Squad {
                    idx: caller_idx,
                    result: raw,
                });
            }
        }

        outcomes
            .into_iter()
            .enumerate()
            .map(|(i, opt)| {
                opt.unwrap_or_else(|| {
                    error!("worker: missing result at index {}", i);
                    MatchOutcome::League(MatchResult {
                        id: String::new(),
                        league_id: 0,
                        league_slug: String::new(),
                        home_team_id: 0,
                        away_team_id: 0,
                        score: Score::new(0, 0),
                        details: None,
                        friendly: false,
                    })
                })
            })
            .collect()
    }
}
