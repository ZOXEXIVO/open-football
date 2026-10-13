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
//!
//! ## The recipe
//!
//! Every figure quoted against this readout is taken the same way, so any two
//! of them can be compared:
//!
//! 1. `cargo build --release` in `.dev/match`, then
//!    `OF_NO_BROWSER=1 OF_NO_LINEUP=1 target/release/dev_match.exe viewer 14 14`.
//!    The harness serves uncapped (`fps_cap` 0) with the readout on, and
//!    cross-origin isolated, so the clock here is fine-grained (about 5 µs)
//!    rather than the 100 µs step above; `OF_NO_BROWSER` keeps a second
//!    viewer off the GPU being measured.
//! 2. `chrome --headless=new --enable-logging=stderr --v=0
//!    --user-data-dir=<fresh absolute path> --disable-gpu-vsync
//!    --disable-frame-rate-limit --autoplay-policy=no-user-gesture-required
//!    --window-size=1920,1080 http://localhost:18002/` for sixty seconds.
//!    A fresh profile every run: Chrome keeps linked programs in it, and a
//!    second run would report a first open it never paid for. Autoplay is
//!    allowed so the soundtrack runs as it does for anyone who has clicked.
//! 3. The page's console comes out as `INFO:CONSOLE` lines on Chrome's
//!    stderr. Read the median of the last ten frame-cost lines. The squad's
//!    own cost is that figure minus the same run with `?squad=off`;
//!    `?grain=full`, `?grain=spare` and `?grain=far` set the finest cut a
//!    man is drawn at, `far` drawing everybody at the coarsest; and
//!    `programs at ready` is the shader bill of the open (see
//!    [`ProgramCensus`]).

use crate::app::bill::MemoryBill;
use crate::app::bringup::{Bringup, Phase};
use crate::app::config::ViewerConfig;
use crate::players::body::Grain;
use bevy::camera::visibility::ViewVisibility;
use bevy::platform::collections::HashSet;
use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::render_resource::{CachedPipelineState, PipelineCache, PipelineDescriptor};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use bevy::shader::ShaderDefVal;

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
    /// How many different mesh assets the drawn ones share between them: the
    /// count 0.20's WebGL2 renderer rebuilds bind groups for, every frame.
    distinct: u32,
    counted: u32,
    /// **What the squad costs in triangles**, and at which grain — reported
    /// once by [`crate::players::actors::Actors::spawn`] and repeated on every
    /// line afterwards. See [`Self::note_geometry`] for why it is told rather
    /// than counted.
    geometry: Option<String>,
    /// How many men on the pitch are drawn at each grain, finest first — see
    /// [`Tailoring`](crate::players::body::tailoring::Tailoring).
    cut: [u32; Grain::ALL.len()],
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
        meshes: Query<(&ViewVisibility, &Mesh3d)>,
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
            let mut assets = HashSet::new();
            for (visibility, mesh) in &meshes {
                total += 1;
                if visibility.get() {
                    drawn += 1;
                    assets.insert(mesh.id());
                }
            }
            cost.meshes = total;
            cost.drawn = drawn;
            cost.distinct = assets.len() as u32;
        }

        if !config.instrumented() {
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

    /// **What the geometry budget came to**, told once at bring-up.
    ///
    /// Told rather than counted, and the reason is a property of the engine
    /// rather than a shortcut: every mesh in this scene is built with
    /// `RenderAssetUsages::RENDER_WORLD`, so the moment it has been extracted
    /// to the render world its vertex data is REMOVED from the main world's
    /// `Assets<Mesh>`. A system here could walk every `Mesh3d` in the world
    /// and would find nothing behind the handles to count. The one place the
    /// figure is knowable is where the meshes are made.
    ///
    /// It exists because the alternative was demonstrated. `SIDES` went 32 to
    /// 128 on 2026-08-30, one outfielder went from ~33,000 triangles to
    /// 377,024, and the line below — which reports mesh ENTITIES, and reported
    /// them accurately throughout — said nothing, because entities are not
    /// what changed. A budget nobody prints is a budget that grows elevenfold
    /// and is found by hand, months later, off a user's complaint.
    pub fn note_geometry(&mut self, summary: String) {
        self.geometry = Some(summary);
    }

    /// How many men on the pitch are drawn at each grain this frame.
    pub fn note_cut(&mut self, cut: [u32; Grain::ALL.len()]) {
        self.cut = cut;
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
    /// Reported from [`crate::recording::loader`] rather than measured here,
    /// because it does not happen on a frame: the fetch lands on the microtask
    /// queue, so the parse falls between two animation frames and shows up in
    /// the frame clock as one enormous `outside`. This is what says whether
    /// that was the recording or the renderer.
    pub fn announce_chunk(index: usize, bytes: usize, milliseconds: f32) {
        Self::announce(&format!(
            "match viewer — chunk {index} opened: {:.1} MB of JSON, envelope in {milliseconds:.0} ms",
            bytes as f32 / (1024.0 * 1024.0),
        ));
    }

    /// The same chunk when the worker read it — the parse figure is the
    /// WORKER'S clock, fetch to flat floats, none of it on a frame. What this
    /// thread pays instead is a copy on arrival and the budgeted fold in
    /// `ChunkLoader::read_on`, and a line here that starts growing back
    /// toward the old envelope numbers means the workshop has quietly stopped
    /// carrying the work.
    pub fn announce_woven(index: usize, bytes: usize, milliseconds: f32) {
        Self::announce(&format!(
            "match viewer — chunk {index} woven off-thread: {:.1} MB of JSON, worker parse {milliseconds:.0} ms",
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
    ///
    /// The bytes ride along with the milliseconds because the two failures
    /// they describe are told apart by nothing else. A frame that is late and
    /// a tab that is about to be killed look identical from outside, and on
    /// the device where the second one happens there is no console to read
    /// either of them in — see [`MemoryBill`].
    pub fn report(&self) -> String {
        let (frame, worst) = self.frame.spread();
        let (main, _) = self.main.spread();
        let (update, _) = self.update.spread();
        let fps = if frame > 0.0 { 1000.0 / frame } else { 0.0 };
        format!(
            "match viewer — {fps:.0} fps · frame {frame:.2} ms (p95 {worst:.1}, \
             worst {spike:.0}) = update {update:.2} + rest of main {rest:.2} \
             + outside {outside:.2} · {drawn}/{meshes} meshes drawn ({distinct} distinct){geometry} \
             · men cut full/spare/far {cut:?} · heap {heap:.0} MiB (peak {peak:.0}) \
             · assets {assets:.0} MiB",
            spike = self.spike,
            rest = (main - update).max(0.0),
            outside = (frame - main).max(0.0),
            drawn = self.drawn,
            meshes = self.meshes,
            distinct = self.distinct,
            cut = self.cut,
            geometry = match &self.geometry {
                Some(summary) => format!(" · {summary}"),
                None => String::new(),
            },
            heap = MemoryBill::mib(MemoryBill::heap()),
            peak = MemoryBill::mib(MemoryBill::peak()),
            assets = MemoryBill::mib(MemoryBill::total()),
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

/// **Which shader programs a session links, and whose frame pays for each.**
///
/// On WebGL2 every pipeline is a program linked synchronously on the main
/// thread, four to six seconds apiece for a material one under ANGLE, which
/// is why [`Bringup`] spreads the scene over courses. This names each
/// pipeline as the cache finishes it — with the shader defs and vertex layout
/// that make it a program of its own — against the phase the page was told
/// about that frame, and totals them at `ready`. Instrumented sessions only.
pub struct ProgramCensus;

impl Plugin for ProgramCensus {
    fn build(&self, app: &mut App) {
        let wanted = app
            .world()
            .get_resource::<ViewerConfig>()
            .is_some_and(ViewerConfig::instrumented);
        let Some(render) = app.get_sub_app_mut(RenderApp).filter(|_| wanted) else {
            return;
        };
        render
            .init_resource::<Linked>()
            .add_systems(ExtractSchedule, Linked::follow_bringup)
            .add_systems(Render, Linked::note.in_set(RenderSystems::Cleanup));
    }
}

/// The census, kept in the render world.
#[derive(Resource, Default)]
struct Linked {
    /// What the page was last told, copied over from the main world.
    phase: Option<Phase>,
    /// Which of the cache's pipelines have been reported, by position.
    reported: Vec<bool>,
    pipelines: usize,
    materials: usize,
    /// Wall time of the frames that linked anything, in milliseconds.
    linking: f32,
    since: Option<Instant>,
    totalled: bool,
}

impl Linked {
    fn follow_bringup(bringup: Extract<Res<Bringup>>, mut linked: ResMut<Linked>) {
        linked.phase = bringup.phase();
    }

    fn note(cache: Res<PipelineCache>, mut linked: ResMut<Linked>) {
        let now = Instant::now();
        let frame = linked
            .since
            .map_or(0.0, |since| (now - since).as_secs_f32() * 1000.0);
        linked.since = Some(now);

        let mut fresh = Vec::new();
        for (index, pipeline) in cache.pipelines().enumerate() {
            if !matches!(pipeline.state, CachedPipelineState::Ok(_)) {
                continue;
            }
            if linked.reported.len() <= index {
                linked.reported.resize(index + 1, false);
            }
            if !std::mem::replace(&mut linked.reported[index], true) {
                fresh.push(Self::describe(&pipeline.descriptor));
            }
        }
        if !fresh.is_empty() {
            let materials = fresh.iter().filter(|(material, _)| *material).count();
            linked.pipelines += fresh.len();
            linked.materials += materials;
            linked.linking += frame;
            FrameCost::announce(&format!(
                "match viewer — {}: {} pipelines linked, {} of them material, in a {:.0} ms frame",
                Self::stage(linked.phase),
                fresh.len(),
                materials,
                frame
            ));
            for (_, line) in &fresh {
                FrameCost::announce(line);
            }
        }
        if linked.phase == Some(Phase::Ready) && !linked.totalled {
            linked.totalled = true;
            FrameCost::announce(&format!(
                "match viewer — programs at ready: {} pipelines, {} material, {:.1} s in the frames that linked them",
                linked.pipelines,
                linked.materials,
                linked.linking / 1000.0
            ));
        }
    }

    fn stage(phase: Option<Phase>) -> String {
        match phase {
            None | Some(Phase::Starting) => "starting".into(),
            Some(Phase::Building(course)) => format!("course {course}/{}", Bringup::COURSES),
            Some(Phase::Recording) => "recording".into(),
            Some(Phase::Squad) => "squad".into(),
            Some(Phase::Ready) => "ready".into(),
        }
    }

    /// One pipeline on one line, and whether it runs a material's fragment
    /// shader — Bevy labels every one of those `*_mesh_pipeline`, and they are
    /// the programs that cost seconds.
    fn describe(descriptor: &PipelineDescriptor) -> (bool, String) {
        let PipelineDescriptor::RenderPipelineDescriptor(render) = descriptor else {
            return (false, "  (compute)".into());
        };
        let label = render.label.as_deref().unwrap_or("unlabelled");
        let layout = render
            .vertex
            .buffers
            .iter()
            .map(|buffer| {
                let locations: Vec<String> = buffer
                    .attributes
                    .iter()
                    .map(|attribute| attribute.shader_location.to_string())
                    .collect();
                format!("{}B@{}", buffer.array_stride, locations.join(","))
            })
            .collect::<Vec<_>>()
            .join(" ");
        let mut defs: Vec<String> = render
            .fragment
            .iter()
            .flat_map(|fragment| fragment.shader_defs.iter().map(Self::def))
            .collect();
        defs.sort();
        (
            label.ends_with("mesh_pipeline"),
            format!("  {label} · vertex {layout} · {}", defs.join(" ")),
        )
    }

    fn def(def: &ShaderDefVal) -> String {
        match def {
            ShaderDefVal::Bool(name, true) => name.to_string(),
            ShaderDefVal::Bool(name, false) => format!("!{name}"),
            ShaderDefVal::Int(name, value) => format!("{name}={value}"),
            ShaderDefVal::UInt(name, value) => format!("{name}={value}"),
        }
    }
}
