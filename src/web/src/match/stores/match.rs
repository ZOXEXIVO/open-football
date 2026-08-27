//! Where a finished match's replay lives on disk, and who turns it into bytes.
//!
//! A recording lands as one gzipped JSON document per five-minute window, plus a
//! small metadata file naming the windows that exist and the stretches of match
//! they cover. The viewer fetches those directly, a chunk at a time — see
//! `crate::r#match::chunk` and `match::recording::loader` on the wasm side.
//!
//! Producing them is split in two on purpose. [`MatchStore::bake`] is pure CPU —
//! serialise, compress — and [`MatchStore::write`] is pure I/O. A match played
//! here does both inside [`MatchStore::store`]. A match played on a remote
//! worker arrives already baked and only needs the second half: the worker never
//! touches a filesystem, it plays and puts bytes on a socket, and compressing a
//! fleet's worth of replays is not work the one machine serving them should be
//! doing on everyone's behalf.
//!
//! Baking on the worker also keeps the two shapes honest. While the wire carried
//! its own derived mirror of a recording, that mirror could — and did — hold
//! more than the files do: the event streams escaped clipping for as long as
//! nobody was comparing the two (`ResultMatchPositionData::finish_retaining`).
//! When the wire *is* the disk bytes there is nowhere for that to hide.

use core::r#match::{MatchResult, RecordingArtifacts, ResultMatchPositionData};
use flate2::Compression;
use flate2::write::GzEncoder;
use log::{debug, error};
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;

const MATCH_DIRECTORY: &str = "match_results";
const CHUNK_DURATION_MS: u64 = 300_000; // 5 minutes per chunk

/// gzip level for everything written here. `best` rather than `fast`: a chunk is
/// compressed once and then served for the life of the save, and on real match
/// data it comes out 17% smaller than `fast` for well under a tenth of a second.
const CHUNK_COMPRESSION: Compression = Compression::best();

pub struct MatchStore;

impl MatchStore {
    pub async fn get_chunk(
        league_slug: &str,
        match_id: &str,
        chunk_number: usize,
    ) -> Option<Vec<u8>> {
        let chunk_file = PathBuf::from(MATCH_DIRECTORY)
            .join(league_slug)
            .join(format!("{}_chunk_{}.json.gz", match_id, chunk_number));

        match tokio::fs::read(&chunk_file).await {
            Ok(bytes) => Some(bytes),
            Err(_) => {
                debug!("Chunk file not found: {}", chunk_file.display());
                None
            }
        }
    }

    pub async fn get_metadata(league_slug: &str, match_id: &str) -> Option<serde_json::Value> {
        let metadata_file = PathBuf::from(MATCH_DIRECTORY)
            .join(league_slug)
            .join(format!("{}_metadata.json", match_id));

        let contents = match tokio::fs::read(&metadata_file).await {
            Ok(bytes) => bytes,
            Err(_) => {
                debug!("Metadata file not found: {}", metadata_file.display());
                return None; // No metadata means no chunks available
            }
        };

        match serde_json::from_slice(&contents) {
            Ok(metadata) => Some(metadata),
            Err(e) => {
                error!("failed to parse metadata for match {}: {}", match_id, e);
                None
            }
        }
    }

    /// Turn a finished recording into the exact bytes that belong on disk.
    ///
    /// Pure CPU and no I/O, so it runs anywhere — including on a match worker,
    /// which has no `match_results` directory and no business having one.
    /// Callers on an async task must hand it to `spawn_blocking`: gzipping a
    /// full-scope track is seconds of work, not microseconds.
    ///
    /// Empty windows produce no chunk. A goals-only recording leaves most
    /// five-minute windows with nothing in them, and the index still has to line
    /// up with the clock — so the chunk keeps its number, there is just no file
    /// behind it, and the viewer knows from the metadata's segments not to ask.
    pub fn bake(data: &ResultMatchPositionData) -> RecordingArtifacts {
        let chunks = data.split_into_chunks(CHUNK_DURATION_MS);
        let chunk_count = chunks.len();

        let baked: Vec<(usize, Vec<u8>)> = chunks
            .iter()
            .enumerate()
            .filter(|(_, chunk)| !chunk.is_empty())
            .map(|(idx, chunk)| (idx, Self::gzip_json(chunk)))
            .collect();

        let mut metadata = serde_json::json!({
            "chunk_count": chunk_count,
            "chunk_duration_ms": CHUNK_DURATION_MS,
            "total_duration_ms": data.max_timestamp()
        });

        // Present only for a clipped recording, and then even when it is empty
        // — a goalless match records nothing at all, and "no segments" has to
        // read differently from "the field isn't there", which is what every
        // recording made before clipping existed looks like.
        if let Some(segments) = data.recorded_segments() {
            metadata["segments"] = serde_json::Value::Array(
                segments
                    .iter()
                    .map(|(start, end)| serde_json::json!([start, end]))
                    .collect(),
            );
        }

        RecordingArtifacts {
            chunks: baked,
            metadata: serde_json::to_vec_pretty(&metadata)
                .expect("metadata is a json! literal and cannot fail to serialise"),
        }
    }

    /// Put baked bytes on disk under `match_results/{league_slug}/`.
    ///
    /// A failure here costs one match its replay, which the viewer already knows
    /// how to say; it must not take the matchday down with it, so every step
    /// logs rather than panics.
    pub async fn write(league_slug: &str, match_id: &str, artifacts: &RecordingArtifacts) {
        let out_dir = PathBuf::from(MATCH_DIRECTORY).join(league_slug);

        if let Err(e) = tokio::fs::create_dir_all(&out_dir).await {
            error!("failed to create {}: {}", out_dir.display(), e);
            return;
        }

        for (idx, bytes) in &artifacts.chunks {
            let chunk_file = out_dir.join(format!("{}_chunk_{}.json.gz", match_id, idx));
            if let Err(e) = tokio::fs::write(&chunk_file, bytes).await {
                error!("failed to write {}: {}", chunk_file.display(), e);
            }
        }

        let metadata_file = out_dir.join(format!("{}_metadata.json", match_id));
        if let Err(e) = tokio::fs::write(&metadata_file, &artifacts.metadata).await {
            error!("failed to write {}: {}", metadata_file.display(), e);
            return;
        }

        debug!(
            "stored {} chunk(s), {} KiB, for match {}",
            artifacts.chunks.len(),
            artifacts.byte_len() / 1024,
            match_id
        );
    }

    /// Bake a result's recording if nobody has yet, then write it.
    pub async fn store(result: MatchResult) {
        let Some(details) = result.details else {
            return;
        };

        let artifacts = match details.recording_artifacts {
            // Played remotely: the worker already did the expensive half.
            Some(baked) => baked,
            // Played here. `bake` is CPU-bound for as long as a full-scope
            // track takes to gzip, so it must not run on a runtime thread.
            None => {
                let data = details.position_data;
                match tokio::task::spawn_blocking(move || Self::bake(&data)).await {
                    Ok(artifacts) => artifacts,
                    Err(e) => {
                        error!("baking the replay for match {} panicked: {}", result.id, e);
                        return;
                    }
                }
            }
        };

        Self::write(&result.league_slug, &result.id, &artifacts).await;
    }

    fn gzip_json<T: Serialize>(value: &T) -> Vec<u8> {
        let raw = serde_json::to_vec(value).expect("a recording chunk always serialises");
        let mut encoder = GzEncoder::new(Vec::new(), CHUNK_COMPRESSION);
        encoder
            .write_all(&raw)
            .expect("gzipping into a Vec cannot fail");
        encoder.finish().expect("gzipping into a Vec cannot fail")
    }
}

/// Baking is the half of storing a replay that a match worker also does, so it
/// has to be exactly right in a process that will never open the files.
///
/// Two things can go quietly wrong. Bytes that are not what the viewer fetches
/// — a chunk keyed off its position in the list rather than its position on the
/// clock, a metadata document missing the segment list — leave a replay that
/// loads to a grey pitch. And artifacts that do not survive bincode leave a
/// worker whose every match arrives blank.
#[cfg(test)]
mod tests {
    use super::*;
    use core::r#match::{RecordingScope, ResultPositionDataItem};
    use flate2::read::GzDecoder;
    use std::io::Read;

    /// A `Vector3` without a `nalgebra` dependency. The recorder's adders want
    /// one and the type has no name inside this crate, so the value is borrowed
    /// back off an item built from plain coordinates. A macro rather than a
    /// function for exactly that reason: nothing here can write the return type.
    macro_rules! v {
        ($x:expr, $y:expr, $z:expr) => {
            ResultPositionDataItem::from_coords(0, [$x, $y, $z]).position
        };
    }

    /// A clipped recording of a match with one goal in it, sampled densely
    /// enough that the chunk either side of the goal is real and the rest are
    /// empty — which is the shape every league match on disk actually has.
    fn recorded(scope: RecordingScope) -> ResultMatchPositionData {
        let mut data = ResultMatchPositionData::new().with_scope(scope);
        let mut t = 30;
        // Twenty minutes: four five-minute windows, with the goal in the third.
        while t <= 1_200_000 {
            if t == 660_000 {
                data.mark_goal(t);
            }
            let drift = (t / 30) as f32;
            data.add_ball_positions(t, v!(drift % 800.0, 272.0, 0.0));
            data.add_player_positions(7, t, v!(drift % 800.0, 200.0, 0.0));
            t += 30;
        }
        data.finish(1_200_000);
        data
    }

    fn gunzip(bytes: &[u8]) -> String {
        let mut out = String::new();
        GzDecoder::new(bytes)
            .read_to_string(&mut out)
            .expect("a chunk is gzip");
        out
    }

    #[test]
    fn a_chunk_is_indexed_by_the_clock_not_by_its_place_in_the_list() {
        let data = recorded(RecordingScope::Goals);
        let artifacts = MatchStore::bake(&data);

        // The goal is at 11:00, so its clip spans the 10:00–15:00 window: chunk
        // 2. The kick-off holds chunk 0, and the whistle at 20:00 falls on the
        // boundary into chunk 4. Nothing is written for the quiet windows, and
        // the indices skip them rather than closing up.
        let indices: Vec<usize> = artifacts.chunks.iter().map(|(idx, _)| *idx).collect();
        assert_eq!(indices, vec![0, 2, 3]);

        let metadata: serde_json::Value =
            serde_json::from_slice(&artifacts.metadata).expect("metadata is json");
        assert_eq!(metadata["chunk_count"], 4);
        assert_eq!(metadata["chunk_duration_ms"], 300_000);
    }

    #[test]
    fn the_metadata_carries_the_segments_the_viewer_seeks_by() {
        let artifacts = MatchStore::bake(&recorded(RecordingScope::Goals));
        let metadata: serde_json::Value =
            serde_json::from_slice(&artifacts.metadata).expect("metadata is json");

        assert_eq!(
            metadata["segments"],
            serde_json::json!([[0, 10_000], [655_000, 665_000], [1_190_000, 1_200_000]]),
        );
    }

    /// A full recording covers everything, and reports so by having no segment
    /// list at all — which the viewer reads as "all of it". An empty list would
    /// mean the opposite, so the difference has to survive baking.
    #[test]
    fn a_full_recording_advertises_no_segment_list() {
        let artifacts = MatchStore::bake(&recorded(RecordingScope::Full));
        let metadata: serde_json::Value =
            serde_json::from_slice(&artifacts.metadata).expect("metadata is json");

        assert!(metadata.get("segments").is_none());
        assert_eq!(artifacts.chunks.len(), 4, "a full recording skips nothing");
    }

    /// The recording a match played with recordings off leaves behind. It still
    /// has to produce metadata — an absent one reads as "not written yet" and
    /// the viewer waits for chunks nobody is coming with.
    #[test]
    fn an_unrecorded_match_bakes_to_an_empty_segment_list_and_no_chunks() {
        let artifacts = MatchStore::bake(&ResultMatchPositionData::empty());
        let metadata: serde_json::Value =
            serde_json::from_slice(&artifacts.metadata).expect("metadata is json");

        assert!(artifacts.chunks.is_empty());
        assert_eq!(metadata["segments"], serde_json::json!([]));
    }

    #[test]
    fn a_chunk_holds_the_json_the_viewer_fetches() {
        let data = recorded(RecordingScope::Goals);
        let chunks = data.split_into_chunks(CHUNK_DURATION_MS);
        let artifacts = MatchStore::bake(&data);

        for (idx, bytes) in &artifacts.chunks {
            assert_eq!(
                gunzip(bytes),
                serde_json::to_string(&chunks[*idx]).expect("the chunk serialises"),
                "chunk {} is not the document the viewer reads",
                idx
            );
        }
    }

    /// What the worker protocol does with them. A worker bakes and never writes,
    /// so bincode is the only thing standing between its CPU and the
    /// coordinator's disk.
    #[test]
    fn artifacts_survive_the_wire() {
        let artifacts = MatchStore::bake(&recorded(RecordingScope::Goals));
        let config = bincode::config::standard();

        let encoded = bincode::serde::encode_to_vec(&artifacts, config).expect("artifacts encode");
        let (back, _): (RecordingArtifacts, _) =
            bincode::serde::decode_from_slice(&encoded, config).expect("and decode");

        assert_eq!(back.chunks, artifacts.chunks);
        assert_eq!(back.metadata, artifacts.metadata);
        assert_eq!(back.byte_len(), artifacts.byte_len());
    }
}
