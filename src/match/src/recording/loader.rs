use crate::app::config::ViewerConfig;
use crate::app::perf::FrameCost;
use crate::recording::playback::{Playback, RecordedSpans};
use crate::recording::replay::{
    ChunkPayload, MatchEvent, RecordingMetadata, ReplayTracks, Sample, StateEntry,
};
use bevy::platform::time::Instant;
use bevy::prelude::*;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::Response;

/// A completed background request, waiting to be folded into the world on the
/// next frame. Browser fetches resolve on the JS microtask queue, which has no
/// access to the ECS, so everything lands in a mailbox first.
enum Delivery {
    Metadata(RecordingMetadata),
    /// The recording is not on disk yet — the match may still be simulating.
    MetadataMissing,
    /// A chunk parsed on this thread — the fallback path, for the machine
    /// whose page will not grant a worker.
    Chunk(usize, ChunkPayload),
    /// A chunk the [`Workshop`]'s worker read: the envelope as values and the
    /// players as one flat array of floats, so the only work left on this
    /// thread is copies.
    Woven(usize, Weave),
    ChunkFailed(usize),
}

/// One chunk as the worker hands it over.
///
/// The players are NOT a map of tracks: they are a single `[t, x, y, z]`-quad
/// float array with a span table over it, because that is the shape that
/// crosses the worker boundary as two buffer transfers instead of a hundred
/// thousand allocations. [`ChunkLoader::read_on`] slices players out of it
/// against the same frame budget the JSON path uses.
struct Weave {
    ball: Vec<Sample>,
    events: Vec<MatchEvent>,
    states: HashMap<u32, Vec<StateEntry>>,
    /// Every player's samples, back to back, four floats per sample.
    positions: Vec<f32>,
    /// `(player id, first sample, sample count)` into `positions`, taken from
    /// the back as they are read.
    spans: Vec<(u32, usize, usize)>,
}

/// What the worker could not flatten: the event log and the state lines, sent
/// back as one small JSON string precisely so the types that already know how
/// to read them — [`MatchEvent`], [`StateEntry`] — go on being the only
/// definition of their format.
#[derive(Deserialize, Default)]
struct Residue {
    #[serde(default)]
    events: Vec<MatchEvent>,
    #[serde(default)]
    states: HashMap<u32, Vec<StateEntry>>,
}

/// A chunk whose envelope is in and whose players are not.
///
/// See [`crate::recording::replay::ChunkPayload`] for why they are separated at
/// all, and [`ChunkLoader::PARSE_BUDGET_MS`] for how much of a frame reading
/// them is allowed to cost.
struct Unread {
    index: usize,
    players: Backlog,
}

/// The two shapes those waiting players come in: raw JSON from the fallback
/// parse, flat floats from the worker. Either way they are read a player at a
/// time under [`ChunkLoader::PARSE_BUDGET_MS`].
enum Backlog {
    Json(Vec<(u32, Box<RawValue>)>),
    Quads {
        positions: Vec<f32>,
        spans: Vec<(u32, usize, usize)>,
    },
}

impl Backlog {
    fn spent(&self) -> bool {
        match self {
            Backlog::Json(players) => players.is_empty(),
            Backlog::Quads { spans, .. } => spans.is_empty(),
        }
    }
}

thread_local! {
    /// The worker, held on the only thread that can own one.
    ///
    /// Not on [`ChunkLoader`]: a `Resource` has to be `Send` and a
    /// `web_sys::Worker` is a handle into the JS heap, which never is. The
    /// outer `Option` is "has hiring been tried", the inner is "did it work" —
    /// a page that refuses workers refuses them for the whole session, and
    /// the refusal must be remembered or every chunk would try again.
    static WORKSHOP: RefCell<Option<Option<Workshop>>> = const { RefCell::new(None) };

    /// Chunks handed to the worker and not yet answered. If the worker itself
    /// dies — a content policy that lets it be created but not run, say — the
    /// error handler fails every one of these, so the loader's ordinary retry
    /// asks for them again down the fallback path instead of waiting forever.
    static COMMISSIONED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
}

/// A plain dedicated Web Worker that fetches and parses chunks so the replay's
/// own thread never has to.
///
/// # Why a worker, and why this shape
///
/// A chunk is five minutes of twenty-three tracks — 4.5 MB of JSON — and
/// parsed here it cost 34–46 ms in one frame, three deep when the read-ahead
/// asked together: the recorded `worst 141` in the frame log, and the one
/// stutter no amount of render tuning could touch. The deferred-players split
/// (see [`ChunkPayload`]) took the samples out of that parse but the envelope
/// scan still walked all of the document on this thread.
///
/// The worker does the whole read — fetch, inflate, parse — off-thread, and
/// hands back the one shape whose crossing is cheap: flat `Float32Array`s,
/// moved by transfer rather than copied by the structured clone. What lands on
/// this thread is a memcpy and a span table; the samples are built from it a
/// player at a time under the same budget the JSON path uses.
///
/// It is a PLAIN worker on purpose. No atomics, no shared memory, no
/// cross-origin isolation — COOP/COEP headers would break the cross-origin
/// player photographs — and no file: the script travels as a blob URL minted
/// from the string below, so the build recipe and the asset list know nothing
/// about it.
struct Workshop {
    worker: web_sys::Worker,
    /// Both closures live exactly as long as the worker does; dropping either
    /// would unhook it mid-session.
    _mail: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _mishap: Closure<dyn FnMut(web_sys::Event)>,
}

impl Workshop {
    /// The worker's whole program. It answers `{index, url}` with either
    /// `{index, ok: false}` or `{index, ok: true, bytes, parse, ball,
    /// positions, spans, meta}`, where `ball` and `positions` are
    /// `[t, x, y, z]` quads, `spans` is `(id, first sample, samples)` triples
    /// over `positions`, and `meta` is the events and states re-serialised
    /// small — so the types that already read them stay their only definition.
    const LOOM: &'static str = r#"
onmessage = async (event) => {
    const { index, url } = event.data;
    try {
        const started = performance.now();
        const response = await fetch(url);
        if (!response.ok) {
            postMessage({ index, ok: false });
            return;
        }
        const body = await response.arrayBuffer();
        const doc = JSON.parse(new TextDecoder().decode(body));
        const quads = (rows) => {
            const flat = new Float32Array((rows ? rows.length : 0) * 4);
            let at = 0;
            for (const row of rows || []) {
                flat[at++] = row[0];
                flat[at++] = row[1];
                flat[at++] = row[2];
                flat[at++] = row.length > 3 ? row[3] : 0;
            }
            return flat;
        };
        const ball = quads(doc.ball);
        const players = doc.players || {};
        const ids = Object.keys(players);
        let total = 0;
        for (const id of ids) total += players[id].length;
        const positions = new Float32Array(total * 4);
        const spans = new Uint32Array(ids.length * 3);
        let cursor = 0;
        ids.forEach((id, slot) => {
            const rows = players[id];
            spans[slot * 3] = Number(id);
            spans[slot * 3 + 1] = cursor;
            spans[slot * 3 + 2] = rows.length;
            let at = cursor * 4;
            for (const row of rows) {
                positions[at++] = row[0];
                positions[at++] = row[1];
                positions[at++] = row[2];
                positions[at++] = row.length > 3 ? row[3] : 0;
            }
            cursor += rows.length;
        });
        const meta = JSON.stringify({
            events: doc.events || [],
            states: doc.states || {},
        });
        postMessage(
            {
                index,
                ok: true,
                bytes: body.byteLength,
                parse: performance.now() - started,
                ball,
                positions,
                spans,
                meta,
            },
            [ball.buffer, positions.buffer, spans.buffer],
        );
    } catch (fault) {
        postMessage({ index, ok: false });
    }
};
"#;

    /// Builds the worker, or answers why not with nothing: a page that blocks
    /// blob workers throws here, and the caller falls back to parsing on this
    /// thread — slower, never wronger.
    fn hire(inbox: &Arc<Mutex<Vec<Delivery>>>, debug: bool) -> Option<Workshop> {
        let source = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(Self::LOOM));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("text/javascript");
        let blob = web_sys::Blob::new_with_str_sequence_and_options(&source, &options).ok()?;
        // The blob URL is deliberately never revoked: the script fetch behind
        // `Worker::new` is asynchronous, and a URL revoked under it is a
        // worker that silently never starts. One short string for the session.
        let url = web_sys::Url::create_object_url_with_blob(&blob).ok()?;
        let worker = web_sys::Worker::new(&url).ok()?;

        let mail = {
            let inbox = Arc::clone(inbox);
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
                let delivery = Self::unpack(event.data(), debug);
                if let Delivery::Woven(index, _) | Delivery::ChunkFailed(index) = &delivery {
                    COMMISSIONED.with(|out| {
                        out.borrow_mut().remove(index);
                    });
                }
                ChunkLoader::deliver(&inbox, delivery);
            })
        };
        worker.set_onmessage(Some(mail.as_ref().unchecked_ref()));

        // A worker that cannot RUN — created fine, refused at execution —
        // surfaces here and nowhere else. Every chunk it was holding is
        // failed back to the loader, whose ordinary retry re-asks down the
        // fallback path, and the workshop is marked closed so nothing else is
        // sent to it.
        let mishap = {
            let inbox = Arc::clone(inbox);
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
                WORKSHOP.with(|shop| {
                    if let Ok(mut shop) = shop.try_borrow_mut() {
                        *shop = Some(None);
                    }
                });
                COMMISSIONED.with(|out| {
                    for index in out.borrow_mut().drain() {
                        ChunkLoader::deliver(&inbox, Delivery::ChunkFailed(index));
                    }
                });
            })
        };
        worker.set_onerror(Some(mishap.as_ref().unchecked_ref()));

        Some(Workshop {
            worker,
            _mail: mail,
            _mishap: mishap,
        })
    }

    /// Hands one chunk to the worker, hiring it on the first call. False when
    /// there is no worker to be had, in which case the caller parses on this
    /// thread exactly as every machine did before there was a workshop.
    fn commission(
        inbox: &Arc<Mutex<Vec<Delivery>>>,
        index: usize,
        url: &str,
        debug: bool,
    ) -> bool {
        WORKSHOP.with(|shop| {
            let mut shop = shop.borrow_mut();
            let Some(workshop) = shop.get_or_insert_with(|| Self::hire(inbox, debug)) else {
                return false;
            };

            let order = js_sys::Object::new();
            let set = |key: &str, value: wasm_bindgen::JsValue| {
                let _ = js_sys::Reflect::set(&order, &wasm_bindgen::JsValue::from_str(key), &value);
            };
            set("index", wasm_bindgen::JsValue::from_f64(index as f64));
            set(
                "url",
                wasm_bindgen::JsValue::from_str(&Self::absolute(url)),
            );
            if workshop.worker.post_message(&order).is_err() {
                *shop = Some(None);
                return false;
            }
            COMMISSIONED.with(|out| {
                out.borrow_mut().insert(index);
            });
            true
        })
    }

    /// A root-relative URL made absolute. The worker resolves relative URLs
    /// against its own script — a `blob:` URL, whose path resolves nothing —
    /// so the page's origin has to be stitched on before the request leaves.
    fn absolute(url: &str) -> String {
        if !url.starts_with('/') {
            return url.to_string();
        }
        web_sys::window()
            .and_then(|window| window.location().origin().ok())
            .map(|origin| format!("{origin}{url}"))
            .unwrap_or_else(|| url.to_string())
    }

    /// One answer off the worker, as a [`Delivery`]. Anything malformed —
    /// which would take a bug in the loom above, not bad data, since the
    /// worker answers `ok: false` for those — degrades to a failed chunk
    /// rather than a wrong one.
    fn unpack(data: wasm_bindgen::JsValue, debug: bool) -> Delivery {
        let take = |name: &str| js_sys::Reflect::get(&data, &wasm_bindgen::JsValue::from_str(name)).ok();
        let index = match take("index").and_then(|value| value.as_f64()) {
            Some(index) if index >= 0.0 => index as usize,
            // No index means the answer cannot be attributed to a request;
            // failing a chunk that nothing asked for is a harmless no-op in
            // `pump`, where failing silently here would strand a real one.
            _ => usize::MAX,
        };
        if !take("ok").and_then(|value| value.as_bool()).unwrap_or(false) {
            return Delivery::ChunkFailed(index);
        }

        let floats = |name: &str| {
            take(name)
                .and_then(|value| value.dyn_into::<js_sys::Float32Array>().ok())
                .map(|array| array.to_vec())
                .unwrap_or_default()
        };
        let ball = Sample::from_quads(&floats("ball"));
        let positions = floats("positions");
        let spans: Vec<(u32, usize, usize)> = take("spans")
            .and_then(|value| value.dyn_into::<js_sys::Uint32Array>().ok())
            .map(|array| array.to_vec())
            .unwrap_or_default()
            .chunks_exact(3)
            .map(|triple| (triple[0], triple[1] as usize, triple[2] as usize))
            .collect();
        let residue: Residue = take("meta")
            .and_then(|value| value.as_string())
            .and_then(|meta| {
                serde_json::from_str(&meta)
                    .inspect_err(|error| error!("bad chunk residue: {error}"))
                    .ok()
            })
            .unwrap_or_default();

        if debug {
            let bytes = take("bytes").and_then(|value| value.as_f64()).unwrap_or(0.0);
            let parse = take("parse").and_then(|value| value.as_f64()).unwrap_or(0.0);
            FrameCost::announce_woven(index, bytes as usize, parse as f32);
        }

        Delivery::Woven(
            index,
            Weave {
                ball,
                events: residue.events,
                states: residue.states,
                positions,
                spans,
            },
        )
    }
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
    /// goal clipping. Read by [`crate::app::bringup::Bringup`], which would
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
                    let players = Backlog::Json(tracks.absorb(payload).into_iter().collect());
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
                Delivery::Woven(index, weave) => {
                    // The worker's twin of the arm above: same envelope-now,
                    // players-over-frames contract, with the players arriving
                    // as floats instead of JSON.
                    tracks.absorb_envelope(weave.ball, weave.events, weave.states);
                    loader.unread.push_back(Unread {
                        index,
                        players: Backlog::Quads {
                            positions: weave.positions,
                            spans: weave.spans,
                        },
                    });
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
    /// [`Self::covers`] is what tells
    /// [`crate::players::actors::Actors::follow_playhead`] that a player with
    /// no samples is genuinely off the pitch rather than still in flight, so
    /// calling a half-read chunk loaded would take the whole squad off the
    /// field for as many frames as the read takes.
    fn read_on(&mut self, tracks: &mut ReplayTracks) {
        let started = Instant::now();
        // Checked after one track rather than before it, so a frame always
        // makes progress: a budget consulted first can be spent by the time
        // it is read and leave the queue standing forever.
        let spent = || (Instant::now() - started).as_secs_f32() * 1000.0 >= Self::PARSE_BUDGET_MS;
        while let Some(chunk) = self.unread.front_mut() {
            match &mut chunk.players {
                Backlog::Json(players) => {
                    while let Some((player_id, samples)) = players.pop() {
                        tracks.absorb_player(player_id, &samples);
                        if spent() {
                            return;
                        }
                    }
                }
                Backlog::Quads { positions, spans } => {
                    while let Some((player_id, first, count)) = spans.pop() {
                        // Sliced defensively: the spans came off a wire, and a
                        // span past the buffer's end is a dropped player, not
                        // a dropped frame.
                        if let Some(quads) = positions.get(first * 4..(first + count) * 4) {
                            tracks.absorb_player_quads(player_id, quads);
                        }
                        if spent() {
                            return;
                        }
                    }
                }
            }
            debug_assert!(chunk.players.spent());
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
        // The worker first — fetch, inflate and parse all land off this
        // thread, and what comes back is flat floats. See [`Workshop`]. The
        // code below survives as the fallback for a page that will not grant
        // one, and as the reference for what the worker's answer means.
        if Workshop::commission(&inbox, index, &url, debug) {
            return;
        }
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
