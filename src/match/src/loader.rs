use crate::config::ViewerConfig;
use crate::playback::Playback;
use crate::replay::{ChunkPayload, RecordingMetadata, ReplayTracks};
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
        for index in current..=current + 2 {
            if index < loader.chunk_count {
                loader.request_chunk(&config, index);
            }
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
        spawn_local(async move {
            let delivery = match Self::get(url).await.and_then(|body| {
                serde_json::from_str::<ChunkPayload>(&body)
                    .inspect_err(|error| error!("bad chunk {index}: {error}"))
                    .ok()
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
