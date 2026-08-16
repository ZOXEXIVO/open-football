use bevy::prelude::Resource;
use serde::de::{Error as DeError, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::fmt;

/// A sample of one entity's position at one instant, in engine units.
///
/// The recorder writes these as bare JSON arrays — `[t, x, y]` on the ground,
/// `[t, x, y, z]` in the air — so the deserialiser below reads a sequence
/// rather than a struct.
#[derive(Clone, Copy)]
pub struct Sample {
    pub t: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl<'de> Deserialize<'de> for Sample {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SampleVisitor;

        impl<'de> Visitor<'de> for SampleVisitor {
            type Value = Sample;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a [timestamp, x, y] or [timestamp, x, y, z] array")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Sample, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let t: f64 = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::invalid_length(0, &self))?;
                let x: f32 = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::invalid_length(1, &self))?;
                let y: f32 = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::invalid_length(2, &self))?;
                let z: f32 = seq.next_element()?.unwrap_or(0.0);

                // Trailing elements would otherwise fail the whole document.
                while seq.next_element::<IgnoredAny>()?.is_some() {}

                Ok(Sample {
                    t: t as u32,
                    x,
                    y,
                    z,
                })
            }
        }

        deserializer.deserialize_seq(SampleVisitor)
    }
}

/// A recorded event line. Only present when the match was recorded with event
/// tracking on; kept because it is the viewer's only window into *why* the
/// engine did what the positions show.
#[derive(Clone, Deserialize)]
pub struct MatchEvent {
    pub timestamp: u64,
    pub category: String,
    pub description: String,
}

/// A player's state machine entering a new state, recorded as
/// `[timestamp, "Group: StateName"]`. The group prefix is dropped on the way
/// in — on a pitch full of markers only the state name fits.
#[derive(Clone)]
pub struct StateEntry {
    pub t: u32,
    pub name: String,
}

impl<'de> Deserialize<'de> for StateEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StateVisitor;

        impl<'de> Visitor<'de> for StateVisitor {
            type Value = StateEntry;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a [timestamp, state] array")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<StateEntry, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let t: f64 = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::invalid_length(0, &self))?;
                let full: String = seq
                    .next_element()?
                    .ok_or_else(|| A::Error::invalid_length(1, &self))?;
                while seq.next_element::<IgnoredAny>()?.is_some() {}

                let name = match full.split_once(": ") {
                    Some((_, state)) => state.to_string(),
                    None => full,
                };
                Ok(StateEntry { t: t as u32, name })
            }
        }

        deserializer.deserialize_seq(StateVisitor)
    }
}

/// The states one player passed through, in order.
#[derive(Default)]
pub struct StateTrack {
    entries: Vec<StateEntry>,
    cursor: usize,
}

impl StateTrack {
    pub fn merge(&mut self, incoming: Vec<StateEntry>) {
        if incoming.is_empty() {
            return;
        }
        if self.entries.is_empty() {
            self.entries = incoming;
            return;
        }
        self.entries.extend(incoming);
        self.entries.sort_by_key(|entry| entry.t);
        self.cursor = 0;
    }

    /// The state in force at `time_ms` — the last one entered before it.
    pub fn name_at(&mut self, time_ms: f64) -> Option<&str> {
        if self.entries.is_empty() || (self.entries[0].t as f64) > time_ms {
            return None;
        }
        let mut index = self.cursor.min(self.entries.len() - 1);
        if (self.entries[index].t as f64) > time_ms {
            index = 0;
        }
        while index + 1 < self.entries.len() && (self.entries[index + 1].t as f64) <= time_ms {
            index += 1;
        }
        self.cursor = index;
        Some(&self.entries[index].name)
    }
}

/// One chunk of a recording as served by `/api/match/{id}/chunk/{n}`.
#[derive(Deserialize)]
pub struct ChunkPayload {
    #[serde(default)]
    pub ball: Vec<Sample>,
    #[serde(default)]
    pub players: HashMap<u32, Vec<Sample>>,
    #[serde(default)]
    pub events: Vec<MatchEvent>,
    #[serde(default)]
    pub states: HashMap<u32, Vec<StateEntry>>,
}

/// Recording metadata as served by `/api/match/{id}/metadata`.
#[derive(Deserialize)]
pub struct RecordingMetadata {
    pub chunk_count: usize,
    pub chunk_duration_ms: u64,
    #[serde(default)]
    pub total_duration_ms: u64,
}

/// A single entity's samples over the whole match, plus the cursor from the
/// last lookup. Playback walks forward in small steps, so remembering where the
/// previous frame landed turns almost every lookup into a one-step advance.
#[derive(Default)]
pub struct Track {
    samples: Vec<Sample>,
    cursor: usize,
    /// A second cursor, for queries that deliberately run ahead of the
    /// playhead.
    ///
    /// The viewer has to know a kick is coming before it happens — a
    /// footballer takes a backswing, and by the time the ball is moving there
    /// is nothing left to draw but the follow-through. The whole recording is
    /// already in memory, so the answer is simply there to be read; what it
    /// must not do is drag the playback cursor forward and back every frame,
    /// which would turn every one of the tens of thousands of normal lookups
    /// into a binary search.
    lookahead: usize,
}

/// How far past the last (or before the first) sample an entity is still drawn.
///
/// The recorder emits a heartbeat sample every 750 ms for everyone on the
/// pitch, so a gap wider than this means the player really is off — substituted
/// or not yet on. Keep it above that heartbeat or stationary players blink.
const PRESENCE_TOLERANCE_MS: f64 = 1000.0;

/// Widest gap between two samples that is still treated as movement.
///
/// The recorder drops samples that repeat a position, so a wider gap than this
/// means the entity was standing still — or that the engine teleported it, as
/// it does when a goal resets the ball to the centre spot. Interpolating across
/// that turns a restart into the ball gliding sixty metres up the pitch.
const INTERPOLATION_GAP_MS: f64 = 200.0;

impl Track {
    /// Fold a chunk's samples in. Chunks normally arrive in playback order, in
    /// which case this is an append; seeking can pull them in out of order, so
    /// fall back to a merge that keeps the timeline sorted.
    pub fn merge(&mut self, incoming: Vec<Sample>) {
        if incoming.is_empty() {
            return;
        }
        if self.samples.is_empty() {
            self.samples = incoming;
            return;
        }
        if incoming[0].t >= self.samples[self.samples.len() - 1].t {
            self.samples.extend(incoming);
            return;
        }

        let mut merged = Vec::with_capacity(self.samples.len() + incoming.len());
        let (mut i, mut j) = (0, 0);
        while i < self.samples.len() && j < incoming.len() {
            if self.samples[i].t <= incoming[j].t {
                merged.push(self.samples[i]);
                i += 1;
            } else {
                merged.push(incoming[j]);
                j += 1;
            }
        }
        merged.extend_from_slice(&self.samples[i..]);
        merged.extend_from_slice(&incoming[j..]);
        self.samples = merged;
        // Both cursors: an out-of-order merge invalidates every index into the
        // old vector, and a stale lookahead is exactly as wrong as a stale
        // playhead.
        self.cursor = 0;
        self.lookahead = 0;
    }

    /// Interpolated position at `time_ms`, or `None` when the entity has no
    /// data anywhere near that instant.
    pub fn position_at(&mut self, time_ms: f64) -> Option<[f32; 3]> {
        let mut cursor = self.cursor;
        let position = self.sample(time_ms, &mut cursor);
        self.cursor = cursor;
        position
    }

    /// The same, for a time deliberately ahead of the playhead — see
    /// [`Track::lookahead`]. Never `Some` past the end of what has streamed in
    /// yet, which is the honest answer: the future has not arrived.
    pub fn position_ahead(&mut self, time_ms: f64) -> Option<[f32; 3]> {
        let mut cursor = self.lookahead;
        let position = self.sample(time_ms, &mut cursor);
        self.lookahead = cursor;
        position
    }

    fn sample(&self, time_ms: f64, cursor: &mut usize) -> Option<[f32; 3]> {
        if self.samples.is_empty() {
            return None;
        }

        let first = self.samples[0].t as f64;
        let last = self.samples[self.samples.len() - 1].t as f64;
        if time_ms < first - PRESENCE_TOLERANCE_MS || time_ms > last + PRESENCE_TOLERANCE_MS {
            return None;
        }

        let index = self.locate(time_ms, *cursor);
        *cursor = index;

        let a = self.samples[index];
        if let Some(b) = self.samples.get(index + 1) {
            let span = (b.t as f64) - (a.t as f64);
            if span > 0.0 && span <= INTERPOLATION_GAP_MS {
                let f = (((time_ms - a.t as f64) / span) as f32).clamp(0.0, 1.0);
                return Some([
                    a.x + (b.x - a.x) * f,
                    a.y + (b.y - a.y) * f,
                    a.z + (b.z - a.z) * f,
                ]);
            }
        }
        Some([a.x, a.y, a.z])
    }

    /// Index of the last sample at or before `time_ms`, starting from
    /// whichever cursor the caller is walking.
    fn locate(&self, time_ms: f64, hint: usize) -> usize {
        let len = self.samples.len();
        let hint = hint.min(len - 1);

        // Fast path: playback advances by a sample or two per frame, so the
        // answer is normally the cursor or its immediate successor. Bail out to
        // the binary search once the scan looks like a seek rather than a step.
        if (self.samples[hint].t as f64) <= time_ms {
            let mut i = hint;
            while i + 1 < len && (self.samples[i + 1].t as f64) <= time_ms {
                i += 1;
                if i - hint > 16 {
                    return self.search(time_ms);
                }
            }
            return i;
        }

        self.search(time_ms)
    }

    fn search(&self, time_ms: f64) -> usize {
        let (mut lo, mut hi) = (0usize, self.samples.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if (self.samples[mid].t as f64) <= time_ms {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        lo
    }
}

/// Everything streamed out of the recording so far.
#[derive(Resource, Default)]
pub struct ReplayTracks {
    pub ball: Track,
    pub players: HashMap<u32, Track>,
    pub states: HashMap<u32, StateTrack>,
    pub events: Vec<MatchEvent>,
    /// Index of the first event not yet written to the console.
    pub next_event: usize,
}

impl ReplayTracks {
    pub fn absorb(&mut self, chunk: ChunkPayload) {
        self.ball.merge(chunk.ball);
        for (player_id, samples) in chunk.players {
            self.players.entry(player_id).or_default().merge(samples);
        }
        for (player_id, entries) in chunk.states {
            self.states.entry(player_id).or_default().merge(entries);
        }
        if !chunk.events.is_empty() {
            self.events.extend(chunk.events);
            self.events.sort_by_key(|event| event.timestamp);
        }
    }
}
