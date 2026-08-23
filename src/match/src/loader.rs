use crate::config::ViewerConfig;
use crate::perf::FrameCost;
use crate::playback::{Playback, RecordedSpans};
use crate::replay::{ChunkPayload, RecordingMetadata, ReplayTracks};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use serde_json::value::RawValue;
use std::collections::{HashSet, VecDeque};
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

/// A chunk whose envelope is in and whose players are not.
///
/// See [`crate::replay::ChunkPayload`] for why they are separated at all, and
/// [`ChunkLoader::PARSE_BUDGET_MS`] for how much of a frame reading them is
/// allowed to cost.
struct Unread {
    index: usize,
    players: Vec<(u32, Box<RawValue>)>,
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
    /// Chunks part-way through being read, oldest first. Normally empty, and
    /// never more than the read-ahead deep.
    unread: VecDeque<Unread>,
    chunk_count: usize,
    chunk_duration_ms: f64,
    metadata_in_flight: bool,
    retry_in: f32,
    /// True once the first chunk is on the pitch.
    pub ready: bool,
    /// True when the recording kept nothing at all — a goalless match under
    /// goal clipping. Read by [`crate::bringup::Bringup`], which would
    /// otherwise hold the loading overlay over an empty pitch waiting for a
    /// squad that is never coming.
    nothing: bool,
}

impl Default for ChunkLoader {
    fn default() -> Self {
        ChunkLoader {
            inbox: Arc::new(Mutex::new(Vec::new())),
            requested: HashSet::new(),
            loaded: HashSet::new(),
            failed: HashSet::new(),
            unread: VecDeque::new(),
            chunk_count: 0,
            chunk_duration_ms: 300_000.0,
            metadata_in_flight: false,
            retry_in: 0.0,
            ready: false,
            nothing: false,
        }
    }
}

impl ChunkLoader {
    /// How much of a frame reading player tracks may take, in milliseconds.
    ///
    /// Three. A sixtieth of a second is 16.7 and this crate's own systems want
    /// a few of those to themselves, so this is about a fifth of the budget —
    /// enough that a whole chunk is in within a handful of frames, small
    /// enough that none of them misses the display. The number that matters is
    /// not this one but the one it replaces: a chunk parsed whole took a
    /// third of a second, in one frame, with nothing else able to run.
    const PARSE_BUDGET_MS: f32 = 3.0;

    /// Whether this recording has anything in it to play at all.
    pub fn nothing_to_play(&self) -> bool {
        self.nothing
    }

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
                            loader.nothing = true;
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
                    // The ball, the states and the event log now; the players
                    // over the next few frames — see `Self::read_on`.
                    let players = tracks.absorb(payload).into_iter().collect();
                    loader.unread.push_back(Unread { index, players });
                    // Whichever chunk lands first is what puts players on the
                    // pitch — normally chunk 0, but a viewer who scrubbed
                    // before it arrived is watching from somewhere else.
                    //
                    // Said on the envelope rather than on the last player: the
                    // ball is in, which is what the camera and the transport
                    // bar have been waiting on, and holding the replay for
                    // twenty-two tracks that are arriving anyway would put the
                    // stall back that splitting them took out.
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

        loader.read_on(&mut tracks);

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

    /// Reads as many of the waiting players' tracks as a frame can afford.
    ///
    /// A chunk is only marked loaded once the last of its players is in.
    /// [`Self::covers`] is what tells [`crate::actors::Actors::follow_playhead`]
    /// that a player with no samples is genuinely off the pitch rather than
    /// still in flight, so calling a half-read chunk loaded would take the
    /// whole squad off the field for as many frames as the read takes.
    fn read_on(&mut self, tracks: &mut ReplayTracks) {
        let started = Instant::now();
        while let Some(chunk) = self.unread.front_mut() {
            while let Some((player_id, samples)) = chunk.players.pop() {
                tracks.absorb_player(player_id, &samples);
                // Checked after one track rather than before it, so a frame
                // always makes progress: a budget consulted first can be
                // spent by the time it is read and leave the queue standing
                // forever.
                if (Instant::now() - started).as_secs_f32() * 1000.0 >= Self::PARSE_BUDGET_MS {
                    return;
                }
            }
            let index = chunk.index;
            self.unread.pop_front();
            self.loaded.insert(index);
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
                serde_json::from_slice::<RecordingMetadata>(&body)
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
                // Timed, because this used to be the one piece of work in the
                // viewer that could stop the page dead. It happens HERE — on
                // the browser's only thread, between two animation frames,
                // with the replay running — and a chunk is five minutes of
                // twenty-three tracks.
                //
                // What is left of it is the ENVELOPE: the ball, the state
                // lines, the event log, and a scan past each player's samples
                // to find where they end. The samples themselves are read a
                // few frames later against a budget — see `ChunkPayload` and
                // `ChunkLoader::read_on`. This number is what says whether the
                // split is holding: it should now be a small fraction of the
                // figure the same line printed before, and the rest of it
                // should have turned into ordinary frames.
                let started = Instant::now();
                let parsed = serde_json::from_slice::<ChunkPayload>(&body)
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
    /// so the browser has already inflated them by the time this sees them.
    ///
    /// **Bytes rather than text**, which is not a detail at a megabyte and a
    /// half. `Response::text` hands back a JavaScript string, and a JavaScript
    /// string is UTF-16: the browser transcodes the UTF-8 it just inflated on
    /// the way out, and `as_string` transcodes it back on the way into
    /// WebAssembly's memory. Two passes over the whole document, both of them
    /// on the main thread, to arrive at the bytes that were already there.
    /// `array_buffer` skips both — what comes back is copied into wasm as a
    /// straight `memcpy` — and `serde_json` is as happy reading a slice as a
    /// string.
    async fn get(url: String) -> Option<Vec<u8>> {
        let window = web_sys::window()?;
        let response: Response = JsFuture::from(window.fetch_with_str(&url))
            .await
            .ok()?
            .dyn_into()
            .ok()?;
        if !response.ok() {
            return None;
        }
        let buffer = JsFuture::from(response.array_buffer().ok()?).await.ok()?;
        Some(js_sys::Uint8Array::new(&buffer).to_vec())
    }
}
