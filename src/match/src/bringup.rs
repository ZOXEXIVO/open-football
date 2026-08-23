//! Building the stadium over several frames instead of one, and telling the
//! page how far along it is.
//!
//! ## What this exists to fix
//!
//! Opening a match for the first time in a browser session froze the tab for
//! the better part of half a minute. Measured on an RTX 3080 Ti against a
//! local server — so with the download and the recording effectively free —
//! the main thread was unavailable for 26.9 s, and 23.7 s of that was four
//! calls to `getProgramParameter(LINK_STATUS)`: the browser blocking while
//! ANGLE translated Bevy's PBR fragment shader to HLSL and D3D compiled it.
//! Four programs, four to six seconds each, and there is nothing to be done
//! about the cost of one of them here — wgpu's WebGL2 backend links a program
//! and asks for its status in the next statement, so the link is synchronous
//! whatever the driver would have been willing to do in the background.
//! (`KHR_parallel_shader_compile` is advertised by the browser and unused by
//! the backend; making use of it is a wgpu change, not one available here.)
//!
//! What CAN be fixed is that three of the four landed in ONE 18.2 s hole. The
//! whole scene was spawned in `Startup`, so every pipeline it needs was queued
//! on the same frame and the render sub-app created them back to back without
//! ever returning to the browser. Nothing repainted, no click was answered,
//! and the loading message on the page could not so much as change its text.
//!
//! So the scene is laid out one course at a time, a course per frame. The
//! shader a course needs is compiled in that frame's render pass and the next
//! frame does not start until the browser has had the thread back: it
//! repaints, it answers the pointer, and the page's loading readout moves on a
//! notch. The total is barely changed — this is not an optimisation, it is the
//! difference between a page that is busy and a page that is dead.
//!
//! ## And why the page is told about it
//!
//! A progress bar only the viewer can fill is the only honest one: the page
//! has no way to know that the turf is down and the stands are not. Each
//! course dispatches a `match-viewer-progress` event on the document naming
//! what it has just finished, and the match page turns that into a phase line
//! and a bar. The last one is `ready`, which is what takes the overlay off —
//! deliberately not the first drawn frame, because the frame the squad first
//! appears on is itself one of the four expensive ones.

use crate::loader::ChunkLoader;
use crate::quality::Quality;
use bevy::prelude::*;

/// How far through the bring-up the scene is.
#[derive(Resource)]
pub struct Bringup {
    /// Updates run before the first course is laid. See
    /// [`Self::WARM_UP`], which explains why laying one on the very first
    /// update would put three of them in the same drawn frame.
    warming: u32,
    /// The course being laid this frame. Counts from one; past
    /// [`Self::COURSES`] the structure is up and only the recording is
    /// outstanding.
    course: usize,
    /// Whether anybody has taken the field yet — set by
    /// [`crate::actors::Actors::take_the_field`], which is where the squad's
    /// own pipelines are queued.
    squad_out: bool,
    /// When the squad came out, on the real clock. See [`Self::SETTLE`].
    settle_at: Option<f32>,
    /// The last phase the page was told about, so nothing is said twice.
    told: Option<Phase>,
}

/// What the page is told the viewer is doing.
///
/// The names travel as strings; the page maps them to its own translated
/// prose, so renaming one here means renaming it in `match/get/index.html`
/// too.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The engine is up and the first frame has not been drawn yet.
    Starting,
    /// Laying the ground, the paint, the goals and the stands — course `at` of
    /// [`Bringup::COURSES`].
    Building(usize),
    /// The structure is up and the recording has not landed.
    Recording,
    /// The recording is in and the squad is being dressed.
    Squad,
    /// There is football on the screen.
    Ready,
}

impl Default for Bringup {
    fn default() -> Self {
        Bringup {
            warming: 0,
            course: 1,
            squad_out: false,
            settle_at: None,
            told: None,
        }
    }
}

impl Bringup {
    /// How many frames the stadium is spread over.
    ///
    /// One per thing that brings a shader with it, which is what sets the
    /// number: the playing surface (normal-mapped and vertex-coloured), the
    /// surround (the same vertex layout WITHOUT the relief, which is a
    /// separate program), the paint, the goals and the stands. Splitting them
    /// finer would buy nothing — a course that queues no new pipeline costs a
    /// frame and returns a frame.
    pub const COURSES: usize = 5;

    /// Updates to let go by before the first course is laid.
    ///
    /// Not a fudge — it is the shape of the runner. Bevy's winit loop runs the
    /// app several times before the browser has drawn anything: the first
    /// update is what creates the window and the surface, so nothing is
    /// rendered off it, and the pipelines every entity spawned so far needs
    /// are all created together in the SECOND update's render pass. Laying a
    /// course during that window puts it in the same frame as the last one,
    /// which is precisely what this file exists to stop — measured, three
    /// shader compiles inside one 10.8 s hole with the courses already split.
    ///
    /// Two, because two is how many it takes: one for the surface, one for the
    /// frame that first draws through it. Getting this wrong costs a shared
    /// frame and nothing else — courses batch, exactly as they used to.
    const WARM_UP: u32 = 2;

    /// How long past the squad taking the field before the page is told the
    /// replay is ready, in seconds of REAL time.
    ///
    /// The kit, the boots and the face are the last materials in the scene,
    /// and the browser stops for about four seconds linking the shader they
    /// need — on the frame AFTER the one that dresses him, because that is
    /// when the render pass first has to draw him. Say "ready" before that and
    /// the overlay comes off directly onto the worst stall of the load, which
    /// is the one moment it was there for.
    ///
    /// A quarter of a second, and measured on the clock rather than in frames
    /// on purpose. Frames are the wrong unit here: Bevy's winit loop can run
    /// several updates inside one browser task, so counting them announced
    /// "ready" three updates and no repaints after the squad appeared — which
    /// is exactly the bug this constant replaced. The clock cannot be fooled
    /// that way: a stall of any length shows up in it, and a load whose
    /// shaders are already cached pays a quarter second and no more.
    const SETTLE: f32 = 0.25;

    /// A run condition: true on the frame this course is due.
    ///
    /// The courses are registered in `Update` in order and chained with
    /// [`Self::pump`] behind them, so exactly one fires per frame.
    pub fn on(course: usize) -> impl Fn(Res<Bringup>) -> bool + Clone {
        move |bringup: Res<Bringup>| bringup.warmed() && bringup.course == course
    }

    /// True while the stadium is still going up. The courses are gated on this
    /// as a group, so a finished bring-up costs one boolean rather than five
    /// run conditions.
    pub fn building(bringup: Res<Bringup>) -> bool {
        bringup.course <= Self::COURSES
    }

    /// Whether the renderer has had time to draw a frame of its own.
    fn warmed(&self) -> bool {
        self.warming >= Self::WARM_UP
    }

    /// Noted by [`crate::actors::Actors::take_the_field`] the first time it
    /// dresses anybody.
    pub fn squad_took_the_field(&mut self) {
        self.squad_out = true;
    }

    /// Advances the course and keeps the page's readout current.
    ///
    /// Runs every frame for the life of the app, and does nothing at all once
    /// `ready` has gone out.
    pub fn pump(
        mut bringup: ResMut<Bringup>,
        mut quality: ResMut<Quality>,
        loader: Res<ChunkLoader>,
        time: Res<Time<Real>>,
    ) {
        if bringup.told == Some(Phase::Ready) {
            return;
        }

        if !bringup.warmed() {
            bringup.warming += 1;
            bringup.announce(Phase::Starting);
            return;
        }

        if bringup.course <= Self::COURSES {
            let laid = bringup.course;
            bringup.course += 1;
            bringup.announce(Phase::Building(laid));
            if bringup.course > Self::COURSES {
                // The shader compiles are behind us; frame times mean
                // something again. See [`Quality::relent`].
                quality.start();
            }
            return;
        }

        if bringup.squad_out {
            let now = time.elapsed_secs();
            let since = *bringup.settle_at.get_or_insert(now);
            let phase = if now - since >= Self::SETTLE {
                Phase::Ready
            } else {
                Phase::Squad
            };
            bringup.announce(phase);
            return;
        }

        // A goalless clip recording keeps nothing, and the loader says so by
        // going ready with no chunk to wait for. Nobody will ever be dressed,
        // so waiting for the squad would hold the overlay over an empty pitch
        // for the rest of the session.
        if loader.nothing_to_play() {
            bringup.announce(Phase::Ready);
            return;
        }

        let phase = if loader.ready {
            Phase::Squad
        } else {
            Phase::Recording
        };
        bringup.announce(phase);
    }

    /// Tells the page, once per phase.
    fn announce(&mut self, phase: Phase) {
        if self.told == Some(phase) {
            return;
        }
        self.told = Some(phase);
        Progress::dispatch(phase, Self::COURSES);
    }
}

/// The one-way channel to the page.
struct Progress;

impl Progress {
    /// The event the match page listens for.
    #[cfg(target_arch = "wasm32")]
    const EVENT: &'static str = "match-viewer-progress";

    #[cfg(target_arch = "wasm32")]
    fn dispatch(phase: Phase, courses: usize) {
        use wasm_bindgen::JsValue;

        let (name, done) = match phase {
            Phase::Starting => ("starting", 0),
            Phase::Building(at) => ("building", at),
            Phase::Recording => ("recording", courses),
            Phase::Squad => ("squad", courses),
            Phase::Ready => ("ready", courses),
        };

        let detail = js_sys::Object::new();
        let set = |key: &str, value: JsValue| {
            let _ = js_sys::Reflect::set(&detail, &JsValue::from_str(key), &value);
        };
        set("phase", JsValue::from_str(name));
        set("done", JsValue::from_f64(done as f64));
        set("total", JsValue::from_f64(courses as f64));

        let init = web_sys::CustomEventInit::new();
        init.set_detail(&detail);

        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict(Self::EVENT, &init) {
            let _ = document.dispatch_event(&event);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn dispatch(_phase: Phase, _courses: usize) {}
}
