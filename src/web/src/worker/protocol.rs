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
///
/// v2: added `Request::Ping` / `Response::Pong` liveness probe.
/// v3: the handshake carries [`RecordingSettings`] and outcomes carry a
/// compressed replay track, so a match played remotely is watchable.
pub const PROTOCOL_VERSION: u32 = 3;

/// What the coordinator wants recorded, sent once per connection.
///
/// A worker has no settings of its own worth honouring: it is a pair of hands,
/// and the machine that will *serve* the replay is the one that gets to say
/// whether there should be one. Sent on the handshake rather than per batch
/// because the coordinator's own switches (`--match-recording-disabled`,
/// `--match-events`, `--match-recording-full`) are read once at startup and
/// never move, and because the worker applies them to process-global state —
/// re-deciding it per batch would be a race dressed up as flexibility.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RecordingSettings {
    /// Record the position track at all. `false` reproduces the pre-v3
    /// behaviour exactly: the worker plays blind and nothing comes back.
    pub positions: bool,
    /// Also record passes, match events and per-player state changes —
    /// the coordinator's `--match-events`.
    pub events: bool,
    /// `RecordingScope::Full` rather than the default `Goals`. Costly: a full
    /// track is ~26× a clipped one, on the wire as well as on disk.
    pub full_scope: bool,
}

impl RecordingSettings {
    /// What a worker assumes before a coordinator has told it otherwise, and
    /// what a coordinator with recordings switched off sends.
    pub fn off() -> Self {
        RecordingSettings {
            positions: false,
            events: false,
            full_scope: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Handshake {
        coordinator_version: String,
        protocol_version: u32,
        recording: RecordingSettings,
    },
    PlayBatch {
        items: Vec<MatchEnvelope>,
    },
    /// Liveness probe sent by the coordinator's health monitor over an
    /// otherwise-idle worker connection. The worker answers
    /// `Response::Pong` immediately; a missing or late reply means the
    /// socket is dead even though no match batch has failed on it yet.
    /// Cheap enough to send every few seconds.
    Ping,
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
    /// Reply to `Request::Ping`. Its mere arrival (in time) is the proof
    /// of life; it carries no payload.
    Pong,
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
///
/// The replay travels *beside* the result rather than inside it.
/// `MatchResultRaw::position_data` is `#[serde(skip)]` and stays that way —
/// it is the right default for every other thing that serialises a result, and
/// the type is serialise-only besides. So the worker lifts the track out,
/// compresses it (see [`recording`](super::recording)) and hangs the blob here;
/// the coordinator puts it back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchOutcome {
    League {
        result: MatchResult,
        /// gzipped bincode of the match's replay track, or `None` when the
        /// coordinator asked for no recording (or the match produced none).
        track: Option<Vec<u8>>,
    },
    Squad {
        idx: usize,
        result: MatchResultRaw,
        track: Option<Vec<u8>>,
    },
}
