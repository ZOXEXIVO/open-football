//! Bincode message envelopes that travel over the raw-TCP frame
//! transport (see `transport.rs`). The coordinator opens one
//! connection per worker entry, sends `Request::Handshake` first, then
//! any number of `Request::PlayBatch`. The worker replies one
//! `Response` per request, in order.

use crate::worker::wire::{LeagueMatchWire, SquadFixtureWire};
use core::r#match::{MatchResult, MatchResultRaw};
use serde::{Deserialize, Serialize};

/// Wire protocol version. Bumped when the on-wire shape changes in a
/// backwards-incompatible way (new required fields, semantic changes
/// to existing fields). The coordinator's host-version check (see
/// `HandshakeResponse::version`) is the primary gate; this is a finer
/// belt-and-braces signal for builds that share a binary version but
/// diverged at the wire layer.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Handshake {
        coordinator_version: String,
        protocol_version: u32,
    },
    PlayBatch {
        items: Vec<MatchEnvelope>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    Handshake {
        version: String,
        protocol_version: u32,
        threads: usize,
        computer_name: String,
        cpu_brand: String,
    },
    /// Worker rejected the handshake (e.g. its own version-compatibility
    /// check failed before serving). Coordinator should mark the entry
    /// `Unreachable` and stop using the connection.
    HandshakeRejected {
        reason: String,
    },
    PlayBatch {
        items: Vec<MatchOutcome>,
    },
    /// Generic failure for a request the worker tried to handle. The
    /// coordinator falls back to the local rayon pool for the affected
    /// batch.
    Error {
        reason: String,
    },
}

/// Per-item envelope inside `Request::PlayBatch`. Two variants cover the
/// two `MatchPlayEnginePool` entry points used by the rest of the
/// engine: league/cup fixtures (`Match`) and raw squad-vs-squad
/// (`play_squads_with_knockout`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchEnvelope {
    League(LeagueMatchWire),
    Squad(SquadFixtureWire),
}

/// Per-item envelope inside `Response::PlayBatch`. Variant must match
/// the input envelope's variant; the coordinator pairs results to
/// requests by input index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchOutcome {
    League(MatchResult),
    Squad { idx: usize, result: MatchResultRaw },
}
