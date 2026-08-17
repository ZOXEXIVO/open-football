use crate::field::Field;
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
    /// The `[start, end]` ranges the recording covers. The game records the
    /// goals and nothing else, so most of a match is a hole; the timeline
    /// greys those out and playback jumps them.
    ///
    /// Absent means the whole match is there — a full recording, or one made
    /// before clipping existed. An empty list is NOT the same thing: it is a
    /// goalless match, where there was nothing to keep.
    #[serde(default)]
    pub segments: Option<Vec<[u64; 2]>>,
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

/// Fastest anything on this pitch can actually travel, in metres per second.
///
/// The guard above tests the wrong thing on its own. It asks how long the gap
/// between two samples is, when what makes a jump a teleport is how far the
/// thing went in that time — and the recorder emits a sample every 30 ms
/// whatever the engine does, so a restart lands between two CONSECUTIVE
/// samples and sails straight through a test keyed on 200 ms.
///
/// Measured over a real match, and not by counting steps — by asking whether
/// each fast step is part of a FLIGHT or stands alone, which is what separates
/// a struck ball from a placed one. Below 45 m/s the fast steps come in runs:
/// 212 of them in the 38-45 band belong to a flight against 43 that do not.
/// Above 45 that reverses completely — 5 belong to a flight, 270 stand alone,
/// each one a goal kick, a throw-in, a corner, a catch or a block, where the
/// engine puts the ball somewhere rather than moving it there.
///
/// So this is not a guess at how hard a footballer can strike a ball. It is
/// the line the engine's own two behaviours fall either side of.
///
/// Players are cut by the same number with a hundredfold margin: nobody covers
/// ground above 8 m/s, and a substitution or a set-piece placement moves one
/// across the pitch inside a single sample — up to 3,270 m/s of implied pace,
/// which used to be drawn as a man skating the length of the field.
const TELEPORT_SPEED: f32 = 45.0;

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
            if span > 0.0 && span <= INTERPOLATION_GAP_MS && !Self::teleported(a, *b, span) {
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

    /// Whether the engine PUT this thing here rather than it having travelled.
    ///
    /// Holding the earlier sample until the playhead passes the later one is
    /// how a teleport gets drawn: as a cut. Sliding between them draws a goal
    /// kick as the ball flying backwards out of the net at two hundred metres
    /// a second, which is the bug this exists to stop — and then the viewer
    /// reads that flight back off the path and spins the ball like a top.
    fn teleported(from: Sample, to: Sample, span: f64) -> bool {
        // `x`/`y` are engine grid units and `z` is already metres — the one
        // place in the crate where the two axes carry different units.
        let across = ((to.x - from.x).hypot(to.y - from.y)) * Field::METERS_PER_UNIT;
        let travelled = across.hypot(to.z - from.z);
        travelled > TELEPORT_SPEED * (span as f32 / 1000.0)
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

#[cfg(test)]
mod cuts {
    use super::*;

    fn track(rows: &[(u32, f32, f32, f32)]) -> Track {
        let mut track = Track::default();
        track.merge(
            rows.iter()
                .map(|&(t, x, y, z)| Sample { t, x, y, z })
                .collect(),
        );
        track
    }

    /// A ball the engine PUT somewhere is cut to, not flown to.
    ///
    /// Measured off a real recording: a goal kick moves the ball about six
    /// metres between two samples thirty milliseconds apart, which the old
    /// guard — keyed on a 200 ms gap — interpolated into a two-hundred-metre-
    /// a-second flight backwards out of the goal. That is the "ball bounces
    /// off the goal" report.
    #[test]
    fn a_restart_is_cut_to_rather_than_flown_to() {
        // Rolling out over the goal line, then placed on the six-yard line.
        let mut ball = track(&[
            (0, 18.5, 314.7, 0.0),
            (30, 1.9, 317.7, 0.0),
            (60, 49.8, 306.6, 0.0),
        ]);

        // Mid-way through the jump the ball is still where it was, not half a
        // goal kick up the pitch.
        let held = ball.position_at(45.0).expect("a sample");
        assert!(
            (held[0] - 1.9).abs() < 1e-3 && (held[1] - 317.7).abs() < 1e-3,
            "interpolated across a teleport: {held:?}"
        );
        // And it is there the moment the playhead reaches it.
        let arrived = ball.position_at(60.0).expect("a sample");
        assert!((arrived[0] - 49.8).abs() < 1e-3);
    }

    /// The hardest strike a footballer produces is still drawn as travel.
    ///
    /// The two populations do not overlap — real steps stop around 40 m/s and
    /// the placements start above 50 — but a cut that swallowed a shot would
    /// be a worse bug than the one it fixes.
    #[test]
    fn a_hard_shot_is_still_interpolated() {
        // 40 m/s: 320 units of travel per second, 9.6 over a 30 ms step.
        let mut ball = track(&[(0, 400.0, 272.0, 0.5), (30, 409.6, 272.0, 0.5)]);
        let middle = ball.position_at(15.0).expect("a sample");
        assert!(
            (middle[0] - 404.8).abs() < 0.05,
            "a legal shot was cut instead of flown: {middle:?}"
        );
    }

    /// Height counts too: the recorder's vertical axis is already in metres
    /// while the other two are grid units, and a ball dropped eight metres
    /// onto a keeper in one sample is a placement however little ground it
    /// covered.
    #[test]
    fn a_vertical_teleport_is_a_teleport() {
        let mut ball = track(&[(0, 400.0, 272.0, 8.9), (30, 400.4, 272.0, 0.0)]);
        let middle = ball.position_at(15.0).expect("a sample");
        assert!(
            (middle[2] - 8.9).abs() < 1e-3,
            "slid down the drop: {middle:?}"
        );
    }

    /// And a player who is walked across the pitch for a set piece is cut to
    /// as well — 3,270 m/s of implied pace in the same recording.
    #[test]
    fn a_set_piece_placement_does_not_skate() {
        let mut player = track(&[(0, 700.0, 250.0, 0.0), (30, 60.0, 300.0, 0.0)]);
        let middle = player.position_at(15.0).expect("a sample");
        assert!(
            (middle[0] - 700.0).abs() < 1e-3,
            "skated the length of the pitch: {middle:?}"
        );
    }
}
