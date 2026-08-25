//! Where a frame actually goes, measured in the browser it is slow in.
//!
//! A replay that "feels laggy when the camera runs" can be any of three very
//! different things, and they are fixed by opposite means:
//!
//! - **The game logic.** Twenty-two actors posed over fifty-odd joints each,
//!   the netting, the labels. Cut by doing less work per frame.
//! - **The render schedule.** Bevy extracting, batching and then *submitting*
//!   the scene. On WebGL2 every draw call is a validated call into the
//!   browser's GL implementation, so this scales with the number of things
//!   drawn rather than with their size. Cut by drawing fewer, bigger things.
//! - **The GPU itself.** Fill rate: samples written, times the shader that
//!   writes them. This scales with the canvas, the multisample count and how
//!   much of the frame the pitch covers, and is entirely indifferent to how
//!   many entities produced it. Cut by writing fewer samples.
//!
//! Guessing between them is how optimisation work gets spent in the wrong
//! place, so this measures instead. Three clocks, all off one monotonic source
//! so they subtract cleanly:
//!
//! ```text
//! frame   |==============================================|  top of one frame
//!         |-- main -----------------------|              |  to the top of the
//!         |   |-- update -------|         |              |  next
//!         |                               |-- outside ---|
//! ```
//!
//! `update` is this crate's own systems. `main - update` is the rest of Bevy's
//! main world — transform propagation, visibility, UI layout, text shaping.
//! `outside` is the render sub-app plus whatever the browser does between two
//! animation frames, which is where both draw submission and any wait on the
//! GPU land.
//!
//! Reading it: a large `update` is our systems; a large `main - update` is
//! usually change detection thrashing something, and UI layout and text
//! re-shaping are the usual culprits; a large `outside` against a high `drawn`
//! count is submission, and a large `outside` against a low one is fill rate.
//!
//! WebAssembly is single-threaded, so none of these overlap and the arithmetic
//! is exact. `Instant` is `performance.now()` here, which Chrome coarsens to
//! about 100 µs on a page that is not cross-origin isolated — hence the
//! window: a median over two seconds of frames is well clear of that step.

use crate::config::ViewerConfig;
use bevy::camera::visibility::ViewVisibility;
use bevy::platform::time::Instant;
use bevy::prelude::*;

/// A fixed window of millisecond samples, kept unsorted and summarised on
/// demand — a frame writes three of these and the readout asks four times a
/// second, so the sort belongs at the reading end.
struct Window {
    samples: [f32; Self::SPAN],
    cursor: usize,
    filled: usize,
}

impl Default for Window {
    fn default() -> Self {
        Window {
            samples: [0.0; Self::SPAN],
            cursor: 0,
            filled: 0,
        }
    }
}

impl Window {
    /// Two seconds at sixty frames — long enough to average out both the
    /// clock's own granularity and the odd frame spent on a chunk arriving.
    const SPAN: usize = 120;

    fn push(&mut self, milliseconds: f32) {
        self.samples[self.cursor] = milliseconds;
        self.cursor = (self.cursor + 1) % Self::SPAN;
        self.filled = (self.filled + 1).min(Self::SPAN);
    }

    /// Median and 95th percentile together, because they are always wanted
    /// together and the sort is the whole cost.
    ///
    /// The median is what the eye reads as the frame rate; the 95th is what it
    /// reads as a stutter, and a shot that is smooth on average but hitches
    /// four times a second is exactly the complaint this exists to tell apart.
    fn spread(&self) -> (f32, f32) {
        if self.filled == 0 {
            return (0.0, 0.0);
        }
        let mut sorted = self.samples[..self.filled].to_vec();
        sorted.sort_by(f32::total_cmp);
        let at = |fraction: f32| sorted[((sorted.len() - 1) as f32 * fraction) as usize];
        (at(0.5), at(0.95))
    }
}

/// The running measurement.
///
/// Always collected — two clock reads a frame, and a census every fifteenth —
/// and only ever *shown* when the page asked for the debug overlay.
#[derive(Resource, Default)]
pub struct FrameCost {
    /// Top of this frame's main schedule. The gap back to the previous one is
    /// the whole frame; the gap forward to [`Self::close`] is Bevy's main
    /// world.
    opened: Option<Instant>,
    entered: Option<Instant>,
    frame: Window,
    main: Window,
    update: Window,
    /// Mesh entities in the world, and how many of them last frame's culling
    /// let through. Sampled rather than counted every frame.
    meshes: u32,
    drawn: u32,
    counted: u32,
    /// The single worst frame since the last console line, and what it cost.
    ///
    /// Separate from the window's own 95th percentile, and not the same
    /// question. A recording streams in while it plays and the chunks are
    /// parsed on this thread — one JSON document per five minutes of match —
    /// so the failure mode that reads as "laggy" is not a low frame rate at
    /// all, it is one frame in a thousand that takes half a second. A window
    /// of a hundred and twenty frames at two hundred a second is half of one
    /// second wide, and a stall that lands outside it leaves no trace.
    spike: f32,
    /// When the last console line went out.
    announced: Option<Instant>,
}

impl FrameCost {
    /// How often the mesh census runs. It is a full pass over every drawable
    /// in the world, which is the one part of this that is not free.
    const CENSUS: u32 = 15;
    /// How often the console line goes out, in seconds. Often enough to watch
    /// a shot change, rare enough not to be why the log scrolls.
    const ANNOUNCE: f32 = 2.0;

    /// Top of the frame, in `First`.
    pub fn open(mut cost: ResMut<FrameCost>) {
        let now = Instant::now();
        if let Some(previous) = cost.opened {
            let frame = (now - previous).as_secs_f32() * 1000.0;
            cost.frame.push(frame);
            cost.spike = cost.spike.max(frame);
        }
        cost.opened = Some(now);
    }

    /// Start of this crate's own systems — registered ahead of the `Update`
    /// chain in [`crate::MatchViewer::start`].
    pub fn enter_update(mut cost: ResMut<FrameCost>) {
        cost.entered = Some(Instant::now());
    }

    /// End of the same, registered after that chain.
    pub fn leave_update(mut cost: ResMut<FrameCost>) {
        let Some(entered) = cost.entered else {
            return;
        };
        let spent = (Instant::now() - entered).as_secs_f32() * 1000.0;
        cost.update.push(spent);
    }

    /// End of the main schedule, in `Last`. The render sub-app runs after
    /// this, so everything from here to the next [`Self::open`] is submission,
    /// the browser and the GPU.
    pub fn close(
        mut cost: ResMut<FrameCost>,
        meshes: Query<&ViewVisibility, With<Mesh3d>>,
        config: Res<ViewerConfig>,
    ) {
        let now = Instant::now();
        if let Some(opened) = cost.opened {
            let main = (now - opened).as_secs_f32() * 1000.0;
            cost.main.push(main);
        }

        cost.counted += 1;
        if cost.counted >= Self::CENSUS {
            cost.counted = 0;
            let mut total = 0;
            let mut drawn = 0;
            for visibility in &meshes {
                total += 1;
                drawn += u32::from(visibility.get());
            }
            cost.meshes = total;
            cost.drawn = drawn;
        }

        if !config.debug {
            return;
        }
        let due = cost
            .announced
            .is_none_or(|last| (now - last).as_secs_f32() >= Self::ANNOUNCE);
        if due {
            cost.announced = Some(now);
            Self::announce(&cost.report());
            cost.spike = 0.0;
        }
    }

    /// The median whole-frame time over the last window, in milliseconds.
    ///
    /// The one number outside this module that anything else reads. `Quality`
    /// decides on the median rather than on the 95th percentile deliberately:
    /// the question it is asking is "is this machine keeping up", and a
    /// hundred-and-twentieth percentile spike is a chunk landing, which
    /// nothing about the render tier would have helped with.
    pub fn typical_frame_ms(&self) -> f32 {
        self.frame.spread().0
    }

    /// What one chunk of the recording cost to parse, and how big it was.
    ///
    /// Reported from [`crate::loader`] rather than measured here, because it
    /// does not happen on a frame: the fetch lands on the microtask queue, so
    /// the parse falls between two animation frames and shows up in the frame
    /// clock as one enormous `outside`. This is what says whether that was the
    /// recording or the renderer.
    pub fn announce_chunk(index: usize, bytes: usize, milliseconds: f32) {
        Self::announce(&format!(
            "match viewer — chunk {index} opened: {:.1} MB of JSON, envelope in {milliseconds:.0} ms",
            bytes as f32 / (1024.0 * 1024.0),
        ));
    }

    /// The console, on the one target that has one.
    #[cfg(target_arch = "wasm32")]
    fn announce(line: &str) {
        web_sys::console::log_1(&wasm_bindgen::JsValue::from_str(line));
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn announce(_line: &str) {}

    /// The whole picture on one line, for the console.
    pub fn report(&self) -> String {
        let (frame, worst) = self.frame.spread();
        let (main, _) = self.main.spread();
        let (update, _) = self.update.spread();
        let fps = if frame > 0.0 { 1000.0 / frame } else { 0.0 };
        format!(
            "match viewer — {fps:.0} fps · frame {frame:.1} ms (p95 {worst:.1}, \
             worst {spike:.0}) = update {update:.1} + rest of main {rest:.1} \
             + outside {outside:.1} · {drawn}/{meshes} meshes drawn",
            spike = self.spike,
            rest = (main - update).max(0.0),
            outside = (frame - main).max(0.0),
            drawn = self.drawn,
            meshes = self.meshes,
        )
    }

    /// The same, cut down to what fits on the transport bar: frames per
    /// second, then the three parts of a frame, then the mesh census.
    pub fn strip(&self) -> String {
        let (frame, _) = self.frame.spread();
        let (main, _) = self.main.spread();
        let (update, _) = self.update.spread();
        let fps = if frame > 0.0 { 1000.0 / frame } else { 0.0 };
        format!(
            "{fps:.0}fps {frame:.0}ms !{spike:.0} u{update:.1} m{main:.1} o{outside:.1} \
             {drawn}/{meshes}",
            spike = self.spike,
            outside = (frame - main).max(0.0),
            drawn = self.drawn,
            meshes = self.meshes,
        )
    }
}
