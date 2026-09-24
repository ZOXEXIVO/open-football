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
//!
//! ⚠ **And "the squad appeared" means DRAWN, not dressed.** The two used to be
//! the same frame and are not any more: the pre-match line-up
//! ([`crate::broadcast::lineup`]) asks for the whole starting eleven to be
//! built the moment the page opens, so a man is dressed while the recording is
//! still in flight and stands `Hidden` for as long as it takes to arrive. The
//! shader his kit needs is not linked when he is built — it is linked the first
//! time a render pass has to DRAW him. Reading "ready" off the dressing put the
//! overlay away eight seconds early and dropped the four-second link into the
//! opening shot of the ceremony, which is the one place in the replay it is
//! guaranteed to be noticed. See [`Self::squad_on_screen`].

use crate::app::bill::MemoryBill;
use crate::app::config::ViewerConfig;
use crate::app::quality::{Footprint, Quality};
use crate::scene::pitch::Stands;
use crate::players::actors::{PlayerActor, Undressed};
use crate::recording::loader::ChunkLoader;
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
    /// Whether a footballer has been DRAWN yet — see [`Self::squad_on_screen`],
    /// which is careful about why being dressed is not the same question.
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
    /// The courses that lay the ground, one per thing that brings a SHADER
    /// with it: the playing surface (normal-mapped and vertex-coloured), the
    /// surround (the same vertex layout WITHOUT the relief, which is a
    /// separate program), the paint, the goals, and the ground the stands sit
    /// on. Splitting those finer would buy nothing — a course that queues no
    /// new pipeline costs a frame and returns a frame.
    const STRUCTURE: usize = 5;

    /// Frames spent per bank of seating: the one it is built on, and one in
    /// which nothing is built at all.
    ///
    /// ⚠ **The idle frame is the second half of the allocation, not padding.**
    /// A bank's upload copy is freed when a LATER submission is retired, not
    /// on the frame that uploads it — see [`Stands`](crate::scene::pitch::Stands),
    /// which carries the mechanism. Raised on consecutive frames, three copies
    /// of the largest mesh in the scene are alive at once; with a frame
    /// between them, two. On wasm32 that difference is permanent, because the
    /// worst instant of the load is the size of the tab for the rest of the
    /// session — see [`MemoryBill`](crate::app::bill::MemoryBill).
    const PER_BANK: usize = 2;

    /// How many frames the stadium is spread over: the structure, then two per
    /// bank.
    ///
    /// A course is "a frame the bring-up is allowed to spend", and there are
    /// three reasons to spend one — a shader link, a large allocation, and
    /// letting a large allocation GO.
    pub const COURSES: usize = Self::STRUCTURE + Stands::BANKS * Self::PER_BANK;

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

    /// **Whether this course raises a bank**: every other one past the
    /// structure, so no bank is ever built beside the previous bank's upload
    /// copy. See [`Self::PER_BANK`].
    ///
    /// One condition rather than a registration per bank. The ladder is the
    /// arithmetic, so a ground with a different number of banks needs nothing
    /// changed here — and there is no list of course numbers to keep in step
    /// with [`Self::COURSES`].
    fn raises_a_bank(course: usize) -> bool {
        course > Self::STRUCTURE
            && course <= Self::COURSES
            && (course - Self::STRUCTURE - 1).is_multiple_of(Self::PER_BANK)
    }

    /// The run condition behind [`Self::raises_a_bank`].
    pub fn raising(bringup: Res<Bringup>) -> bool {
        bringup.warmed() && Self::raises_a_bank(bringup.course)
    }

    /// Whether the renderer has had time to draw a frame of its own.
    fn warmed(&self) -> bool {
        self.warming >= Self::WARM_UP
    }

    /// **Has a footballer been drawn yet**, which is the question the settle
    /// clock is actually asking.
    ///
    /// ⚠ **Not "has anybody been dressed".** Dressing a man queues his
    /// materials; it does not compile the shader they need, because nothing has
    /// asked to draw him yet. The link happens in the render pass of the first
    /// frame he is VISIBLE, and with the line-up ceremony in front of the match
    /// those two frames are seconds apart — the eleven are built as the page
    /// opens and shown when the recording lands. Asked the wrong question the
    /// overlay came off with an empty pitch behind it and the four-second link
    /// fell into the ceremony's first shot.
    ///
    /// Undressed men are excluded because a body is what carries the meshes: a
    /// man with no meshes cannot queue a pipeline however visible he is.
    fn squad_on_screen(
        squad: &Query<&Visibility, (With<PlayerActor>, Without<Undressed>)>,
    ) -> bool {
        squad
            .iter()
            .any(|visibility| *visibility != Visibility::Hidden)
    }

    /// Advances the course and keeps the page's readout current.
    ///
    /// Runs every frame for the life of the app, and does nothing at all once
    /// `ready` has gone out.
    pub fn pump(
        mut bringup: ResMut<Bringup>,
        mut quality: ResMut<Quality>,
        loader: Res<ChunkLoader>,
        config: Res<ViewerConfig>,
        time: Res<Time<Real>>,
        squad: Query<&Visibility, (With<PlayerActor>, Without<Undressed>)>,
    ) {
        if bringup.told == Some(Phase::Ready) {
            return;
        }
        // Carried onto every phase the page is told about. It is the one
        // decision in this viewer that changes the whole scene and the one
        // nobody can read off the screen, and this is where a device with no
        // console gets to see it — see [`Footprint::probe`].
        let footprint = quality.footprint();

        if !bringup.warmed() {
            bringup.warming += 1;
            bringup.announce(Phase::Starting, footprint);
            return;
        }

        if bringup.course <= Self::COURSES {
            let laid = bringup.course;
            bringup.course += 1;
            bringup.announce(Phase::Building(laid), footprint);
            if bringup.course > Self::COURSES {
                // The shader compiles are behind us; frame times mean
                // something again. See [`Quality::relent`].
                quality.start();
            }
            return;
        }

        // Registered behind the whole `Update` chain, so this is the frame's
        // own answer and not the previous one's: the ceremony stands the line
        // up and the replay reveals whoever is on the pitch well before this
        // runs. Latched, because a man can go back off — a substitute, or the
        // ceremony handing the pitch over — and the shader stays linked.
        bringup.squad_out |= Self::squad_on_screen(&squad);

        if bringup.squad_out {
            let now = time.elapsed_secs();
            let since = *bringup.settle_at.get_or_insert(now);
            let phase = if now - since >= Self::SETTLE {
                Phase::Ready
            } else {
                Phase::Squad
            };
            bringup.announce(phase, footprint);
            return;
        }

        // A goalless clip recording keeps nothing, and the loader says so by
        // going ready with no chunk to wait for. Nobody will ever be dressed,
        // so waiting for the squad would hold the overlay over an empty pitch
        // for the rest of the session.
        //
        // `?squad=off` is the same situation arrived at deliberately — a
        // bisection knob, see [`ViewerConfig::squad`] — and it has to be named
        // here or the overlay never comes off the one scene somebody is
        // holding a failing phone to look at.
        if loader.nothing_to_play() || config.squad_is_off() {
            bringup.announce(Phase::Ready, footprint);
            return;
        }

        let phase = if loader.ready {
            Phase::Squad
        } else {
            Phase::Recording
        };
        bringup.announce(phase, footprint);
    }

    /// Tells the page, once per phase.
    ///
    /// **Every phase is also a memory reading**, and that is the whole of the
    /// instrument this crate has on a device with no console. A tab killed for
    /// its memory leaves nothing behind, but the phase line and the figures
    /// beside it are on the SCREEN until the reload takes them, so whoever is
    /// holding the phone can read which course the scene died on and what it
    /// was holding when it did. See [`MemoryBill`].
    fn announce(&mut self, phase: Phase, footprint: Footprint) {
        if self.told == Some(phase) {
            return;
        }
        self.told = Some(phase);
        Progress::dispatch(phase, Self::COURSES, footprint);
        // The whole bill once, where the scene has stopped growing and
        // everything in it has been made. Not on every phase: it is eight
        // figures, and eight figures printed nine times is a log nobody reads.
        if phase == Phase::Ready {
            MemoryBill::announce(&MemoryBill::line());
        }
    }
}

/// The one-way channel to the page.
struct Progress;

impl Progress {
    /// The event the match page listens for.
    #[cfg(target_arch = "wasm32")]
    const EVENT: &'static str = "match-viewer-progress";

    #[cfg(target_arch = "wasm32")]
    fn dispatch(phase: Phase, courses: usize, footprint: Footprint) {
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
        // **What the viewer is holding, at the moment it says so.** In
        // megabytes, because the page shows this to a person and a person
        // reads megabytes. See [`Bringup::announce`] for why every phase
        // carries it.
        set(
            "footprint",
            JsValue::from_str(match footprint {
                Footprint::Handheld => "handheld",
                Footprint::Roomy => "roomy",
            }),
        );
        set(
            "heap",
            JsValue::from_f64(MemoryBill::mib(MemoryBill::heap()) as f64),
        );
        set(
            "peak",
            JsValue::from_f64(MemoryBill::mib(MemoryBill::peak()) as f64),
        );
        set(
            "assets",
            JsValue::from_f64(MemoryBill::mib(MemoryBill::total()) as f64),
        );

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
    fn dispatch(_phase: Phase, _courses: usize, _footprint: Footprint) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every bank is raised, and no two on consecutive courses.**
    ///
    /// The whole point of [`Bringup::PER_BANK`] is the gap: a bank built on
    /// the frame after another one is built beside that one's upload copy,
    /// which is the instant that sets the tab's permanent high-water mark on
    /// wasm32. The count and the spacing are the two halves of that, and both
    /// fall out of arithmetic rather than a list of course numbers — so this
    /// is what stops a change to either constant from silently dropping a
    /// stand or putting two back together.
    #[test]
    fn the_banks_go_up_one_at_a_time_with_a_frame_between_them() {
        let raising: Vec<usize> = (1..=Bringup::COURSES)
            .filter(|course| Bringup::raises_a_bank(*course))
            .collect();

        assert_eq!(
            raising.len(),
            Stands::BANKS,
            "{raising:?} does not raise every bank exactly once"
        );
        for pair in raising.windows(2) {
            assert!(
                pair[1] - pair[0] >= Bringup::PER_BANK,
                "banks on courses {} and {} are too close together",
                pair[0],
                pair[1]
            );
        }
        // …and the structure is laid before any of them, so no bank shares a
        // frame with a shader link.
        assert!(raising[0] > Bringup::STRUCTURE);
    }

    /// Nothing is raised once the stadium is up, so a long match cannot walk
    /// off the end of the plan.
    #[test]
    fn no_bank_is_raised_past_the_last_course() {
        assert!(!Bringup::raises_a_bank(Bringup::COURSES + 1));
        assert!(!Bringup::raises_a_bank(0));
    }
}
