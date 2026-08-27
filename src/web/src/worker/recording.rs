//! Moving a match recording between a worker and the coordinator.
//!
//! The replay track — every ball and player sample the engine kept, plus the
//! pass/event/state streams when `--match-events` is on — is the one part of a
//! match result that does not simply serde its way across the wire.
//! [`ResultMatchPositionData`] is serialise-only, and on purpose: its
//! `Serialize` writes the compact shape the viewer's chunk files use, which
//! nothing reads back. So the track travels as [`RecordingWire`], a plain
//! derived mirror built from [`RecordingParts`], and it travels **compressed**.
//!
//! Compression is not decoration here. Measured over a season of a-league
//! fixtures under the default [`RecordingScope::Goals`], a match's track is
//! ~525 KB of JSON / ~165 KB gzipped; bincode is denser still. A batch is tens
//! of matches on one frame, so the difference between shipping this raw and
//! shipping it deflated is the difference between a comfortable frame and one
//! that argues with [`MAX_FRAME_BYTES`](super::transport::MAX_FRAME_BYTES).

use core::r#match::{
    MatchEventData, PassEventData, PlayerStateEntry, RecordingParts, RecordingScope,
    ResultMatchPositionData, ResultPositionDataItem,
};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};

/// Wire image of a finished recording.
///
/// Deliberately primitive: `[f32; 3]` rather than a `Vector3`, `Vec<(k, v)>`
/// rather than a `HashMap`, a `u8` rather than a [`RecordingScope`]. Nothing
/// here needs nalgebra, and a sequence of pairs is both smaller and faster
/// under bincode than a map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingWire {
    ball: Vec<(u64, [f32; 3])>,
    players: Vec<(u32, Vec<(u64, [f32; 3])>)>,
    passes: Vec<(u64, u32, u32)>,
    events: Vec<(u64, String, String)>,
    player_states: Vec<(u32, Vec<(u64, String)>)>,
    track_events: bool,
    track_positions: bool,
    scope: u8,
    segments: Vec<(u64, u64)>,
}

impl RecordingWire {
    fn from_recording(data: ResultMatchPositionData) -> Self {
        let RecordingParts {
            ball,
            players,
            passes,
            events,
            player_states,
            track_events,
            track_positions,
            scope,
            segments,
        } = data.into_parts();

        RecordingWire {
            ball: ball
                .iter()
                .map(|item| (item.timestamp, item.coords()))
                .collect(),
            players: players
                .into_iter()
                .map(|(id, samples)| {
                    (
                        id,
                        samples
                            .iter()
                            .map(|item| (item.timestamp, item.coords()))
                            .collect(),
                    )
                })
                .collect(),
            passes: passes
                .into_iter()
                .map(|p| (p.timestamp, p.from_player_id, p.to_player_id))
                .collect(),
            events: events
                .into_iter()
                .map(|e| (e.timestamp, e.category, e.description))
                .collect(),
            player_states: player_states
                .into_iter()
                .map(|(id, entries)| {
                    (
                        id,
                        entries
                            .into_iter()
                            .map(|entry| (entry.timestamp, entry.state))
                            .collect(),
                    )
                })
                .collect(),
            track_events,
            track_positions,
            scope: scope.as_u8(),
            segments,
        }
    }

    fn into_recording(self) -> ResultMatchPositionData {
        let RecordingWire {
            ball,
            players,
            passes,
            events,
            player_states,
            track_events,
            track_positions,
            scope,
            segments,
        } = self;

        ResultMatchPositionData::from_parts(RecordingParts {
            ball: ball
                .into_iter()
                .map(|(t, p)| ResultPositionDataItem::from_coords(t, p))
                .collect(),
            players: players
                .into_iter()
                .map(|(id, samples)| {
                    (
                        id,
                        samples
                            .into_iter()
                            .map(|(t, p)| ResultPositionDataItem::from_coords(t, p))
                            .collect(),
                    )
                })
                .collect::<HashMap<_, _>>(),
            passes: passes
                .into_iter()
                .map(|(timestamp, from_player_id, to_player_id)| {
                    PassEventData::new(timestamp, from_player_id, to_player_id)
                })
                .collect(),
            events: events
                .into_iter()
                .map(|(timestamp, category, description)| MatchEventData {
                    timestamp,
                    category,
                    description,
                })
                .collect(),
            player_states: player_states
                .into_iter()
                .map(|(id, entries)| {
                    (
                        id,
                        entries
                            .into_iter()
                            .map(|(timestamp, state)| PlayerStateEntry { timestamp, state })
                            .collect(),
                    )
                })
                .collect::<HashMap<_, _>>(),
            track_events,
            track_positions,
            scope: RecordingScope::from_u8(scope),
            segments,
        })
    }
}

/// gzip level for the track blob. `fast` rather than `best`: this runs on the
/// worker's critical path between the final whistle and the reply frame, and
/// the samples are quantised floats — the last few percent of ratio is not
/// worth the milliseconds, unlike in `MatchStore`, which writes files once and
/// serves them forever.
const TRACK_COMPRESSION: Compression = Compression::fast();

/// Compress a finished recording for the wire.
///
/// `None` means "there is nothing to send" — a recorder that was never sampled
/// (`empty()`), which is what a worker produces when the coordinator has
/// recordings switched off.
///
/// That is not the same as sending an empty blob, and the difference is
/// load-bearing. A track with no samples but `track_positions` set and a full
/// scope advertises *no* segment list, and the viewer reads an absent segment
/// list as "all of it is recorded" — it would then sit on the loading notice
/// waiting for chunks nobody wrote. `None` leaves the coordinator's own
/// `empty()` in place, which reports the empty segment list that makes the
/// viewer say so instead.
pub fn encode(data: ResultMatchPositionData) -> Option<Vec<u8>> {
    if data.is_empty() {
        return None;
    }

    let wire = RecordingWire::from_recording(data);
    let raw = bincode::serde::encode_to_vec(&wire, bincode::config::standard()).ok()?;

    let mut encoder = GzEncoder::new(Vec::new(), TRACK_COMPRESSION);
    encoder.write_all(&raw).ok()?;
    encoder.finish().ok()
}

/// Inverse of [`encode`]. `None` on any corruption — a match whose replay
/// arrives unreadable is still a match with a score, so the caller drops the
/// track and keeps the result rather than failing the batch.
pub fn decode(blob: &[u8]) -> Option<ResultMatchPositionData> {
    let mut raw = Vec::new();
    GzDecoder::new(blob).read_to_end(&mut raw).ok()?;
    let (wire, _): (RecordingWire, _) =
        bincode::serde::decode_from_slice(&raw, bincode::config::standard()).ok()?;
    Some(wire.into_recording())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `Vector3` without a `nalgebra` dependency. The recorder's adders want
    /// one and the type has no name inside this crate, so the value is borrowed
    /// back off an item built from plain coordinates. A macro rather than a
    /// function for exactly that reason: nothing here can write the return type.
    macro_rules! v {
        ($x:expr, $y:expr, $z:expr) => {
            ResultPositionDataItem::from_coords(0, [$x, $y, $z]).position
        };
    }

    /// Build a recording the way a match does: samples arrive, a goal marks a
    /// clip, the whistle finishes it.
    fn recorded() -> ResultMatchPositionData {
        let mut data = ResultMatchPositionData::new().with_scope(RecordingScope::Goals);
        let mut t = 0;
        while t <= 60_000 {
            let drift = (t / 30) as f32;
            data.add_ball_positions(t, v!(drift, 272.0, 1.5));
            data.add_player_positions(7, t, v!(drift, 200.0, 0.0));
            data.add_player_positions(11, t, v!(400.0 - drift, 300.0, 0.0));
            if t == 30_000 {
                data.mark_goal(t);
            }
            t += 30;
        }
        data.finish_retaining(60_000, &[]);
        data
    }

    #[test]
    fn a_recording_survives_the_round_trip() {
        let original = recorded();
        let segments = original.recorded_segments().map(|s| s.to_vec());
        let max_ts = original.max_timestamp();
        let mut ids = original.get_player_ids();
        ids.sort_unstable();
        let ball_at_goal = original.get_ball_position_at(30_000);
        let average_7 = original.player_average_position(7);

        let blob = encode(original).expect("a sampled recording encodes");
        let back = decode(&blob).expect("and decodes");

        assert!(!back.is_empty(), "the track came back empty");
        assert_eq!(
            back.recorded_segments().map(|s| s.to_vec()),
            segments,
            "the segment list is what the viewer seeks by — it has to survive"
        );
        assert_eq!(back.max_timestamp(), max_ts);
        let mut back_ids = back.get_player_ids();
        back_ids.sort_unstable();
        assert_eq!(back_ids, ids);
        assert_eq!(back.get_ball_position_at(30_000), ball_at_goal);
        assert_eq!(back.player_average_position(7), average_7);
    }

    /// With `--match-events` on, the recording carries three more streams. They
    /// ride the same blob; a round trip that quietly dropped them would leave
    /// the viewer with movement and no explanation of it.
    #[test]
    fn the_event_streams_ride_along_too() {
        let mut data = ResultMatchPositionData::new_with_tracking();
        data.add_ball_positions(0, v!(420.0, 272.0, 0.0));
        data.add_player_positions(7, 0, v!(400.0, 200.0, 0.0));
        data.add_pass_event(1_000, 7, 11);
        data.add_match_event(2_000, "goal", "Someone scored".to_string());
        data.add_player_state(7, 3_000, 42, &"Running");
        data.finish_retaining(4_000, &[]);

        let expected = serde_json::to_string(&data).expect("the viewer shape serialises");
        let back = decode(&encode(data).expect("encodes")).expect("decodes");

        assert_eq!(
            serde_json::to_string(&back).expect("and still serialises"),
            expected,
            "the rebuilt recording must write the same chunk the original would"
        );
    }

    /// The case this whole module exists to stop producing: an unsampled
    /// recorder has nothing to send, and says so with `None` rather than a blob
    /// that decodes to an empty track.
    #[test]
    fn an_unsampled_recording_encodes_to_nothing() {
        assert!(encode(ResultMatchPositionData::empty()).is_none());
    }

    #[test]
    fn a_corrupt_blob_decodes_to_nothing_rather_than_panicking() {
        assert!(decode(&[0u8, 1, 2, 3, 4]).is_none());
    }
}
