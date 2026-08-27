//! Bincode message envelopes that travel over the raw-TCP frame
//! transport (see `transport.rs`). The coordinator opens one
//! connection per worker entry, sends `Request::Handshake` first, then
//! any number of `Request::PlayBatch`. Every request is answered by exactly one
//! terminal `Response`, in order — a `PlayBatch` may be preceded by any number
//! of [`Response::Artifacts`] frames carrying that batch's replays, so a reader
//! consumes frames until a terminal one arrives.

use crate::worker::wire::{LeagueMatchWire, SquadFixtureWire};
use core::r#match::{MatchResult, MatchResultRaw, RecordingArtifacts};
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
/// v4: the replay crosses as the finished chunk files rather than a wire-only
/// mirror of the recorder, and each one gets its own frame
/// ([`Response::Artifacts`]) instead of sharing the batch's.
pub const PROTOCOL_VERSION: u32 = 4;

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
    /// One match's baked replay, sent ahead of the `PlayBatch` frame it belongs
    /// to. `pos` indexes the request's `items`.
    ///
    /// A frame of its own, one per match, because a batch is `2 × threads`
    /// matches and its replays used to have to fit *together* under
    /// [`MAX_FRAME_BYTES`](super::transport::MAX_FRAME_BYTES). They did not
    /// always: a worker that overran the budget dropped the surplus replays on
    /// the floor to save the results, which is the right trade to make and the
    /// wrong situation to be in. Framed one at a time there is no budget to
    /// overrun — a single match's replay is a hundred and seventy kilobytes
    /// clipped, and even a `--match-recording-full` track is well inside the
    /// cap.
    Artifacts {
        pos: usize,
        artifacts: RecordingArtifacts,
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
/// The replay does not travel inside the result. `MatchResultRaw::position_data`
/// is `#[serde(skip)]` and stays that way — it is the right default for every
/// other thing that serialises a result, and the type is serialise-only besides.
/// It travels as its own [`Response::Artifacts`] frame, matched back to the
/// outcome by input index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MatchOutcome {
    League { result: MatchResult },
    Squad { idx: usize, result: MatchResultRaw },
}
