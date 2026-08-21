use crate::config::ViewerConfig;
use crate::perf::FrameCost;
use crate::playback::{Playback, RecordedSpans};
use crate::replay::{ChunkPayload, RecordingMetadata, ReplayTracks};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::Response;

/// A completed background request, waiting to be folded into the world on the
/// next frame. Browser fetches resolve on the JS microtask queue, which has no
/// access to the ECS, so everything lands in a mailbox first.
enum Delivery {
    Metadata(RecordingMetadata),
    /// The recording is not on disk yet — the match may still be simulating.
    MetadataMissing,
    Chunk(usize, ChunkPayload),
    ChunkFailed(usize),
}

/// How long to wait before asking again for a recording that is not ready.
const METADATA_RETRY_SECONDS: f32 = 3.0;

/// Streams the recording in, one chunk at a time, keeping just far enough ahead
/// of the playhead that seeking stays responsive without pulling a whole match
/// into memory up front.
#[derive(Resource)]
pub struct ChunkLoader {
    inbox: Arc<Mutex<Vec<Delivery>>>,
    requested: HashSet<usize>,
    loaded: HashSet<usize>,
    /// Chunks that have already failed once. A second failure is taken as a
    /// hole in the recording rather than a blip, and the loader stops asking —
    /// the alternative is one request per frame for the rest of the match.
    failed: HashSet<usize>,
    chunk_count: usize,
    chunk_duration_ms: f64,
    metadata_in_flight: bool,
    retry_in: f32,
    /// True once the first chunk is on the pitch.
    pub ready: bool,
}

impl Default for ChunkLoader {
    fn default() -> Self {
        ChunkLoader {
            inbox: Arc::new(Mutex::new(Vec::new())),
            requested: HashSet::new(),
            loaded: HashSet::new(),
            failed: HashSet::new(),
            chunk_count: 0,
            chunk_duration_ms: 300_000.0,
            metadata_in_flight: false,
            retry_in: 0.0,
            ready: false,
        }
    }
}

impl ChunkLoader {
    /// Kicks off the metadata request that everything else waits on.
    pub fn bootstrap(mut loader: ResMut<ChunkLoader>, config: Res<ViewerConfig>) {
        loader.request_metadata(&config);
    }

    /// Folds finished requests into the world and keeps the read-ahead topped
    /// up around the playhead.
    pub fn pump(
        mut loader: ResMut<ChunkLoader>,
        mut tracks: ResMut<ReplayTracks>,
        mut playback: ResMut<Playback>,
        mut spans: ResMut<RecordedSpans>,
        config: Res<ViewerConfig>,
        time: Res<Time>,
    ) {
        let deliveries: Vec<Delivery> = match loader.inbox.lock() {
            Ok(mut inbox) => inbox.drain(..).collect(),
            Err(_) => Vec::new(),
        };

        for delivery in deliveries {
            match delivery {
                Delivery::Metadata(metadata) => {
                    loader.metadata_in_flight = false;
                    loader.chunk_count = metadata.chunk_count.max(1);
                    loader.chunk_duration_ms = metadata.chunk_duration_ms.max(1) as f64;
                    if playback.duration_ms <= 0.0 && metadata.total_duration_ms > 0 {
                        playback.duration_ms = metadata.total_duration_ms as f64;
                    }
                    if let Some(ranges) = metadata.segments {
                        spans.set(
                            ranges
                                .into_iter()
                                .map(|[start, end]| (start as f64, end as f64))
                                .collect(),
                        );
                        // A goalless match kept nothing. Nothing will ever
                        // arrive, so stop waiting for it — the timeline is
                        // grey end to end and that is the whole story.
                        if spans.nothing_recorded() {
                            loader.ready = true;
                        }
                        // Open on the first goal rather than on kickoff, which
                        // is now a part of the match nobody recorded.
                        if let Some(start) = spans.next_start(playback.time_ms) {
                            if !spans.covers(playback.time_ms) {
                                playback.time_ms = start;
                                playback.seeked = true;
                            }
                        }
                    }
                }
                Delivery::MetadataMissing => {
                    loader.metadata_in_flight = false;
                    loader.retry_in = METADATA_RETRY_SECONDS;
                }
                Delivery::Chunk(index, payload) => {
                    tracks.absorb(payload);
                    loader.loaded.insert(index);
                    // Whichever chunk lands first is what puts players on the
                    // pitch — normally chunk 0, but a viewer who scrubbed
                    // before it arrived is watching from somewhere else.
                    if !loader.ready {
                        loader.ready = true;
                        playback.playing = true;
                    }
                }
                Delivery::ChunkFailed(index) => {
                    if loader.failed.insert(index) {
                        loader.requested.remove(&index);
                    }
                }
            }
        }

        if loader.chunk_count == 0 {
            if loader.retry_in > 0.0 {
                loader.retry_in -= time.delta_secs();
            } else if !loader.metadata_in_flight {
                loader.request_metadata(&config);
            }
            return;
        }

        // Two chunks of read-ahead. One is enough at normal speed, but the dev
        // harness plays back at up to 16x and a backgrounded tab can jump a
        // chunk boundary in a single frame.
        let current = loader.chunk_index(playback.time_ms);
        // Plus whichever chunk holds the next clip. On a goals-only recording
        // the playhead crosses the gap in one frame (`Playback::advance`), and
        // the clip it lands in can be twenty chunks away — read-ahead measured
        // in adjacent chunks would never reach it, and the replay would sit on
        // the last frame of the previous goal waiting.
        let upcoming = spans
            .next_start(playback.time_ms)
            .map(|start| loader.chunk_index(start));

        for index in (current..=current + 2).chain(upcoming) {
            if index >= loader.chunk_count {
                continue;
            }
            // Windows with no clip in them have no file behind them — asking
            // costs a round trip and a 404.
            let from = index as f64 * loader.chunk_duration_ms;
            if !spans.intersects(from, from + loader.chunk_duration_ms) {
                continue;
            }
            loader.request_chunk(&config, index);
        }
    }

    fn chunk_index(&self, time_ms: f64) -> usize {
        (time_ms.max(0.0) / self.chunk_duration_ms) as usize
    }

    /// Whether the recording around `time_ms` has actually arrived. An entity
    /// with no samples there is only genuinely off the pitch if it has — until
    /// then it is just data in flight.
    pub fn covers(&self, time_ms: f64) -> bool {
        self.loaded.contains(&self.chunk_index(time_ms))
    }

    fn request_metadata(&mut self, config: &ViewerConfig) {
        self.metadata_in_flight = true;
        let inbox = Arc::clone(&self.inbox);
        let url = config.metadata_url();
        spawn_local(async move {
            let delivery = match Self::get(url).await.and_then(|body| {
                serde_json::from_str::<RecordingMetadata>(&body)
                    .inspect_err(|error| error!("bad recording metadata: {error}"))
                    .ok()
            }) {
                Some(metadata) => Delivery::Metadata(metadata),
                None => Delivery::MetadataMissing,
            };
            Self::deliver(&inbox, delivery);
        });
    }

    fn request_chunk(&mut self, config: &ViewerConfig, index: usize) {
        if !self.requested.insert(index) {
            return;
        }
        let inbox = Arc::clone(&self.inbox);
        let url = config.chunk_url(index);
        let debug = config.debug;
        spawn_local(async move {
            let delivery = match Self::get(url).await.and_then(|body| {
                // Timed, because this is the one piece of work in the viewer
                // that can stop the page dead. A chunk is five minutes of
                // twenty-three tracks and it is parsed HERE — on the browser's
                // only thread, between two animation frames, with the replay
                // running. Nothing else in the frame is within two orders of
                // magnitude of it, so when a viewer reports a freeze this is
                // the first number to ask for. See `perf`.
                let started = Instant::now();
                let parsed = serde_json::from_str::<ChunkPayload>(&body)
                    .inspect_err(|error| error!("bad chunk {index}: {error}"))
                    .ok();
                if debug {
                    let spent = (Instant::now() - started).as_secs_f32() * 1000.0;
                    FrameCost::announce_chunk(index, body.len(), spent);
                }
                parsed
            }) {
                Some(payload) => Delivery::Chunk(index, payload),
                None => Delivery::ChunkFailed(index),
            };
            Self::deliver(&inbox, delivery);
        });
    }

    fn deliver(inbox: &Arc<Mutex<Vec<Delivery>>>, delivery: Delivery) {
        if let Ok(mut inbox) = inbox.lock() {
            inbox.push(delivery);
        }
    }

    /// The chunks are stored gzipped and served with `Content-Encoding: gzip`,
    /// so the browser hands back plain text here.
    async fn get(url: String) -> Option<String> {
        let window = web_sys::window()?;
        let response: Response = JsFuture::from(window.fetch_with_str(&url))
            .await
            .ok()?
            .dyn_into()
            .ok()?;
        if !response.ok() {
            return None;
        }
        JsFuture::from(response.text().ok()?)
            .await
            .ok()?
            .as_string()
    }
}
