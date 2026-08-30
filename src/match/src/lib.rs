//! Bevy/WebAssembly viewer for matches recorded by the `core` crate.
//!
//! The web crate serves the match page, hands this module a canvas and a JSON
//! description of the fixture, and gets out of the way: fetching the recorded
//! position chunks, rendering the pitch in 3D and driving playback all happen
//! here, so the replay is one artefact rather than a Rust half and a JavaScript
//! half that have to agree with each other.
//!
//! ## Where things live
//!
//! Seven groups, each with its own `mod.rs` saying what it is for:
//!
//! - `app` — the scaffolding between the browser tab and the replay: the
//!   config the page hands over, staged bringup, the quality ladder, the size
//!   the scene is drawn at and what a frame costs.
//! - `recording` — the recorded match: its shape, the fetch that brings it
//!   in a chunk at a time, and the playhead everything is drawn from.
//! - `scene` — the stadium, which is there before the teams are.
//! - `players` — the twenty-two and the football.
//! - `broadcast` — the camera, what it is pointed at, and the two ceremonies
//!   whose shots are written rather than followed.
//! - `ui` — the transport bar drawn over the picture, mouse and touch.
//! - `art` — the images and glyphs the crate paints for itself, since
//!   nothing is loaded over the network.
//! - `sound` — the ball being struck, synthesised in the browser for the
//!   same reason `art` paints its own pixels.
//!
//! This file is the wiring: it holds the entry point the page calls and the
//! one schedule every system in those groups is registered into, which is the
//! only place their order relative to each other is stated.

mod app;
mod art;
mod broadcast;
mod players;
mod recording;
mod scene;
mod sound;
mod ui;

use crate::app::bringup::Bringup;
use crate::app::config::ViewerConfig;
use crate::app::perf::FrameCost;
use crate::app::quality::Quality;
use crate::app::stage::Stage;
use crate::art::typeface::Typeface;
use crate::broadcast::camera::{CameraFlight, CameraOrbit, CameraZoom, TvCamera};
use crate::broadcast::changeover::ChangeoverShot;
use crate::broadcast::cut::CutFade;
use crate::broadcast::focus::{CameraSubject, FocusRing};
use crate::broadcast::lineup::Lineup;
use crate::players::actors::{Actors, BallState};
use crate::players::aftermath::Aftermath;
use crate::players::portrait::Portraits;
use crate::recording::loader::ChunkLoader;
use crate::recording::playback::{EventLog, Playback, RecordedSpans};
use crate::recording::replay::ReplayTracks;
use crate::scene::net::Netting;
use crate::scene::pitch::{Bank, Pitch};
use crate::scene::sky::Sky;
use crate::sound::matchday::{Soundtrack, Speakers};
use crate::ui::timeline::{DebugOverlay, Timeline};
use crate::ui::touch::{FlightPad, TouchControls, TouchDevice, TouchDrive, TouchGesture};
use bevy::asset::AssetMetaCheck;
use bevy::log::{Level, LogPlugin};
use bevy::prelude::*;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct MatchViewer;

#[wasm_bindgen]
impl MatchViewer {
    /// Entry point for the match page. `config` is the JSON form of
    /// [`ViewerConfig`]; the app takes over the canvas it names and runs until
    /// the page goes away.
    #[wasm_bindgen(js_name = start)]
    pub fn start(config: String) {
        console_error_panic_hook::set_once();

        let config: ViewerConfig = match serde_json::from_str(&config) {
            Ok(config) => config,
            Err(error) => {
                web_sys::console::error_1(&JsValue::from_str(&format!(
                    "match viewer: unusable config — {error}"
                )));
                return;
            }
        };

        let duration_ms = config.match_time_ms;
        let debug = config.debug;

        // The orbit drag needs the right and wheel buttons, and the browser
        // answers those with a context menu and autoscroll. Winit would
        // suppress them, but only via the same switch that swallows the
        // keyboard, and the page keeps the keyboard — so they are claimed
        // by hand here. This call had no caller, which is the other half of
        // the orbit never having been wired up.
        CameraOrbit::claim_pointer_buttons(&config.canvas);
        // Same again for the arrow keys, which fly the camera and would
        // otherwise scroll the page under it.
        CameraFlight::claim_flight_keys(&config.canvas);

        // Before anything is drawn, so the camera can be built already
        // carrying the answer.
        let quality = Quality::probe();

        // How often the app runs at all. The browser holds the page to its
        // display, and past the cap that is only ever pictures nobody can
        // tell apart, paid for in GPU heat — a 240 Hz panel would spend
        // double the frame budget of the cap itself. Capped, the event loop
        // wakes on its own drift-free timer (`scheduled start + wait`, so the
        // cadence averages the cap exactly) and on NOTHING else — the
        // `react_to_*` flags stay off because a pointer move would otherwise
        // wake an extra update per event, and a camera drag is precisely when
        // the cap must hold. Input loses nothing: events queue and the next
        // tick reads them, at most one tick late.
        //
        // Zero means uncapped — the `.dev/match` harness asks for that,
        // because it is the measuring instrument and a capped instrument
        // reads the cap instead of the scene. See `ViewerConfig::fps_cap`.
        let pace = if config.fps_cap > 0.0 {
            let tick = bevy::winit::UpdateMode::Reactive {
                wait: std::time::Duration::from_secs_f32(1.0 / config.fps_cap),
                react_to_device_events: false,
                react_to_user_events: false,
                react_to_window_events: false,
            };
            bevy::winit::WinitSettings {
                focused_mode: tick,
                // The same tick out of focus: the browser already throttles a
                // hidden tab's timers, and a visible-but-unfocused replay is
                // still a replay someone is watching.
                unfocused_mode: tick,
            }
        } else {
            bevy::winit::WinitSettings::default()
        };

        App::new()
            .insert_resource(pace)
            .add_plugins(
                DefaultPlugins
                    .set(WindowPlugin {
                        primary_window: Some(Window {
                            canvas: Some(config.canvas.clone()),
                            fit_canvas_to_parent: true,
                            // The page owns the keyboard: swallowing F5 and
                            // friends inside a replay would be hostile.
                            prevent_default_event_handling: false,
                            ..default()
                        }),
                        ..default()
                    })
                    // Nothing is loaded over the network by the asset server —
                    // the only asset is the typeface `Typeface` compiles in —
                    // so asking for `.meta` sidecars would only produce 404s.
                    .set(AssetPlugin {
                        meta_check: AssetMetaCheck::Never,
                        ..default()
                    })
                    .set(LogPlugin {
                        level: Level::WARN,
                        ..default()
                    }),
            )
            // After `DefaultPlugins`, which is what creates `Assets<Font>`, and
            // before any of the spawns below put text on the screen.
            .add_plugins(Typeface)
            // Only ever seen if the dome somehow is not: `Sky` is carried by
            // the lens and covers the frame. Held at the gradient's zenith so
            // that if it ever does show, it shows as more sky.
            .insert_resource(ClearColor(Color::srgb(0.030, 0.048, 0.098)))
            // Asked BEFORE the app is built and inserted here rather than
            // initialised as a system, because `TvCamera::spawn` reads it: a
            // tier settled on the first frame costs nothing to adopt, where
            // one settled on the tenth costs a pipeline recompile. See
            // `quality`.
            .insert_resource(quality)
            .insert_resource(config)
            .insert_resource(Playback::new(duration_ms))
            .init_resource::<ReplayTracks>()
            .init_resource::<ChunkLoader>()
            .init_resource::<RecordedSpans>()
            .init_resource::<BallState>()
            .init_resource::<Aftermath>()
            .init_resource::<DebugOverlay>()
            // Always collected, never in the way: two clock reads a frame and
            // a mesh census every fifteenth. It is only *shown* when the page
            // asked for the debug overlay — see `perf`, which is the thing
            // that turns "laggy" into a number worth acting on.
            .init_resource::<FrameCost>()
            // Before `Startup`, because `TvCamera::spawn` is pointed at the
            // image this owns and startup systems have no order between them
            // worth relying on. See `stage`.
            .init_resource::<Stage>()
            .init_resource::<CameraZoom>()
            // `TvCamera::follow_play` takes `Res<CameraOrbit>` and
            // `CameraOrbit::handle_drag` takes `ResMut<CameraOrbit>`, but
            // nothing ever inserted it — so the first `Update` tick
            // failed parameter validation with "Resource does not exist"
            // and the WASM viewer aborted before rendering a frame. The
            // orbit landed next to the zoom in `camera.rs` and its
            // registration was missed here.
            .init_resource::<CameraOrbit>()
            .init_resource::<CameraFlight>()
            // Which of the twenty-two the camera has been asked to follow.
            // Read by `TvCamera::follow_play`, by the ring on the grass and by
            // the reset chip, so it exists from the first frame whether or not
            // anybody ever clicks a player. See `focus`.
            .init_resource::<CameraSubject>()
            // Filled at startup from the config's substitutions; read by
            // `TvCamera::follow_play` on every frame, so it exists from the
            // first one whether or not the match had a change in it.
            .init_resource::<ChangeoverShot>()
            // The two teams walked out before the first whistle. Registered
            // here as well as filled by `Lineup::arm` at startup, because
            // `TvCamera::follow_play` and `Actors::take_the_field` both take
            // it as a `Res` and `Startup` systems have no order between them
            // worth relying on — the same reason `Stage` is above.
            .init_resource::<Lineup>()
            // The touch half of the same controls. `TouchDrive` is read by
            // `CameraFlight::steer` on every frame whether or not anything has
            // ever been touched, so it is registered unconditionally — the
            // orbit's own missing registration is the cautionary tale a few
            // lines up.
            .init_resource::<TouchDevice>()
            .init_resource::<TouchDrive>()
            .init_resource::<TouchGesture>()
            .init_resource::<FlightPad>()
            .init_resource::<Bringup>()
            // How far through the dip between two clips the picture is. Read
            // every frame whether or not the recording has any holes in it at
            // all, so it exists from the first one. See `cut`.
            .init_resource::<CutFade>()
            // What the ball has already been heard doing, and whether anybody
            // wants to hear it at all. Registered here rather than opened at
            // startup on purpose: the audio engine behind it is not created
            // until the replay is actually running, so a viewer who never
            // presses play never makes the browser open one. See `sound`.
            .init_resource::<Soundtrack>()
            // …and its other half, which holds JavaScript handles and so
            // cannot be a `Resource` at all — `Speakers` is the one non-send
            // thing in the app.
            .init_non_send::<Speakers>()
            .add_systems(
                Startup,
                (
                    Sky::spawn,
                    TvCamera::spawn,
                    // The window camera and the sheet the replay is shown on,
                    // which is not the same camera that draws it — see
                    // `stage`.
                    Stage::spawn,
                    Actors::spawn,
                    // Hidden until a player is picked. Built here rather than
                    // then for the reason `FlightPad::spawn` is: a marker
                    // assembled on the frame it is first wanted is a mesh and
                    // a material queued in the middle of a click.
                    FocusRing::spawn,
                    // Cuts the config's substitutions into one shot per
                    // stoppage. Startup rather than per-frame because the
                    // answer cannot change: the recording is fixed by the
                    // time the page hands it over.
                    ChangeoverShot::arm,
                    // …and stands the two elevens up, off the team sheets.
                    // Startup for the same reason: who started is fixed by the
                    // time the page hands the document over.
                    Lineup::arm,
                    Timeline::spawn,
                    // Hidden until the replay first cuts, and built here for
                    // the reason the flight stick is: a sheet assembled on the
                    // frame it is first wanted is a texture uploaded in the
                    // middle of the moment it exists to cover.
                    CutFade::spawn,
                    ChunkLoader::bootstrap,
                    // Hidden until a finger arrives — see `FlightPad::refresh`.
                    // Spawned here rather than then, because a control built on
                    // the frame it is first grabbed misses that grab.
                    FlightPad::spawn,
                    // After winit has adopted the canvas, or the focus it
                    // takes is the focus this just set.
                    CameraFlight::focus_canvas,
                ),
            )
            // The stadium, a course per frame rather than all of it on the
            // first one. Each of these queues a render pipeline the browser
            // then blocks for seconds compiling, and running them on separate
            // frames is what gives the page the main thread back in between —
            // to repaint, to answer a click, and to move its own loading
            // readout on. See `bringup`, which has the measurements.
            .add_systems(
                Update,
                (
                    Pitch::lay_turf.run_if(Bringup::on(1)),
                    Pitch::lay_surround.run_if(Bringup::on(2)),
                    Pitch::paint_markings.run_if(Bringup::on(3)),
                    Pitch::raise_goals.run_if(Bringup::on(4)),
                    Pitch::build_stands.run_if(Bringup::on(5)),
                )
                    .chain()
                    .run_if(Bringup::building)
                    // Ahead of the replay's own systems, so the course laid
                    // this frame is drawn this frame.
                    .before(FrameCost::enter_update),
            )
            // Behind everything, so the phase the page is told about is the
            // one that has just finished rather than the one about to start.
            .add_systems(Update, Bringup::pump.after(FrameCost::leave_update))
            // **The two teams walked out**, and the one pair of systems in
            // this file whose place in the frame is pinned by name rather than
            // by position in the chain below.
            //
            // ⚠ **`.chain()` does not reach inside a nested tuple.** The chain
            // orders the ELEMENTS of the tuple it is called on; an element
            // that is itself a tuple becomes an unordered group, and the
            // systems in it end up ordered against nothing. Both of these were
            // written that way to stay inside Bevy's maximum arity and both
            // landed in the wrong half of the frame: `pose` ran BEFORE
            // `Actors::follow_playhead`, so the line it stood the men in was
            // overwritten by the recording on the same frame. It looked
            // correct for a fortunate reason — the playhead is parked at zero
            // for the whole ceremony and a recording has no sample there to
            // overwrite it with — and gave itself away the moment the parked
            // time was moved off zero.
            //
            // So they are named here instead, which says what each one
            // actually needs and cannot be broken by an arity limit:
            //
            // - **`hold`** behind every handler that can be a viewer asking
            //   for the football (the keyboard is the last of them, and the
            //   transport bar and the pick are chained ahead of it) and in
            //   front of the playhead it holds still.
            // - **`pose`** behind the recording, which is what it overrides,
            //   and in front of `animate`, which poses whatever it leaves.
            // - **`wear_the_name`** behind `hold`, which is what decides
            //   whether the ceremony has the picture this frame, and behind
            //   `take_the_field`, which is what spawns the print in the first
            //   place — a man dressed this frame wears his name this frame.
            .add_systems(
                Update,
                (
                    Lineup::hold
                        .after(Playback::handle_keyboard)
                        .before(Playback::advance),
                    Lineup::pose
                        .after(Actors::take_the_field)
                        .before(Actors::animate),
                    Lineup::wear_the_name
                        .after(Lineup::hold)
                        .after(Actors::take_the_field),
                    // **Ahead of the fold**, so a request that comes due this
                    // frame is on the wire before the answer to the last one
                    // is painted onto a face — the two are the halves of one
                    // budget and belong in one frame's order, not two.
                    //
                    // Named here for the same reason the line-up's systems
                    // are: the group below is at Bevy's twenty-tuple limit,
                    // and nesting a pair inside it would leave both ordered
                    // against nothing at all.
                    Portraits::ask.before(Portraits::attach),
                ),
            )
            // **The soundtrack**, named here for the same reason the pair
            // above are: both of the twenty-system groups below are full, and
            // what these two need is a place in the frame rather than a place
            // in a list.
            //
            // - **the mute chip** ahead of the paint, so a click on it is
            //   answered on the frame it happened rather than the next one.
            // - **the ball** behind `Actors::follow_playhead`, which is what
            //   settles `BallState` — where the ball is, how fast it is going
            //   and the strike that is coming — and in front of
            //   `Playback::end_frame`, which clears the `seeked` flag it reads
            //   to know its idea of the ball's velocity is worthless.
            .add_systems(
                Update,
                (
                    Timeline::handle_sound.before(Timeline::paint_chips),
                    Soundtrack::follow_playhead
                        .after(Actors::follow_playhead)
                        .after(Timeline::handle_sound)
                        .before(Playback::end_frame),
                ),
            )
            .add_systems(
                Update,
                // Split into two nested groups purely to stay inside
                // Bevy's maximum tuple arity — `.chain()` still orders the
                // whole thing end to end, so the ordering notes below mean
                // exactly what they say across the boundary.
                (
                    // The clock either side of the whole chain, so what the
                    // readout calls `update` is exactly this crate's own
                    // systems and nothing of Bevy's.
                    FrameCost::enter_update,
                    (
                        // First of everything, so a window resize or a step
                        // down the resolution ladder lands on the frame it was
                        // decided rather than being drawn once at the old size.
                        Stage::fit,
                        ChunkLoader::pump,
                        // Beside the chunk loader because it is the same kind
                        // of thing: a fetch that started when the page did,
                        // landing whenever it lands. A face arriving in the
                        // third minute changes nothing about the frame it
                        // arrives on — see `portrait`.
                        Portraits::attach,
                        Timeline::handle_toggle,
                        Timeline::handle_seek,
                        // Ahead of `Playback::advance`, so a change of speed
                        // applies to the frame it was asked for. Registered
                        // here rather than with the debug systems below: the
                        // speed chip is part of the transport bar in the
                        // game too, and a button whose handler never runs is
                        // worse than no button.
                        Timeline::handle_speed,
                        // Ahead of every camera system below, so a click on
                        // the reset chip lands on the frame it happened.
                        Timeline::handle_camera_reset,
                        // The click on the pitch itself, and deliberately
                        // BEFORE the playhead moves: what the pointer was
                        // aimed at is the frame that was on the screen, and
                        // that frame was drawn from last update's positions.
                        // Testing against this update's would ask the viewer
                        // to lead a running player. See `focus`.
                        CameraSubject::handle_pick,
                        Playback::handle_keyboard,
                        Playback::advance,
                        Actors::follow_playhead,
                        // Straight after it, because it is what decides who is
                        // on: a man is built when the playhead comes within a
                        // few seconds of his first recorded sample, so the
                        // thirty-six on the two team sheets are not all
                        // assembled before the first frame. See
                        // `Actors::take_the_field`.
                        Actors::take_the_field,
                        // Between the playhead moving and the bodies being
                        // posed off it: `animate` reads the mood, and reading
                        // last frame's would leave every reaction a frame
                        // behind a scrub.
                        Aftermath::follow_playhead,
                        Actors::animate,
                        // Straight after, so the dive `animate` has just read
                        // out of the recording is on the body the same frame.
                        Actors::carry_body,
                        // And after that, because a shadow is placed off where
                        // the body it belongs to has just been put — including
                        // how far off the ground.
                        Actors::cast_shadows,
                        // After the bodies have moved and before anything
                        // reads the subject: this is what copies the followed
                        // player's position out for the camera and walks the
                        // shot in and out of the close-up.
                        CameraSubject::settle,
                        // Beside it, off the same facts and for the same
                        // reason: the substitution shot aims at where the men
                        // coming on have just been put, and `follow_play`
                        // below reads what it writes.
                        ChangeoverShot::settle,
                        // Straight after it, off the same two facts — where he
                        // is standing and how far the shot has closed.
                        FocusRing::follow,
                        // After `follow_playhead`, which is what moves the
                        // ball: the netting is deformed by wherever the ball
                        // has just been put, so a frame's lag here would show
                        // the mesh trailing the ball through it.
                        Netting::ripple,
                    ),
                    (
                        // Ahead of the gestures that read it, so the frame a
                        // touch device announces itself is the frame its
                        // controls exist.
                        TouchControls::watch,
                        // Beside the mouse handlers below because they are the
                        // same controls, and ahead of `follow_play` for the same
                        // reason: a gesture has to land on the frame it happened.
                        TouchControls::handle_gestures,
                        FlightPad::handle_touch,
                        // Ahead of `follow_play`, which reads the orbit — so a
                        // drag lands on the same frame it happened rather than
                        // the next one. Never registered at all before, so the
                        // camera could not be turned.
                        CameraOrbit::handle_drag,
                        CameraZoom::handle_wheel,
                        TvCamera::follow_play,
                        // After `follow_play` rather than before it: on the
                        // frame the rig takes off, flight seeds itself from the
                        // broadcast position that system has just written, so
                        // the first key press continues the shot instead of
                        // cutting.
                        CameraFlight::steer,
                        // Straight after the camera moves, so the stand the
                        // rig has just walked into is gone on the same frame
                        // it entered rather than flashing for one.
                        Bank::cull,
                        // Same frame, same reason: the dome is carried by the
                        // lens, and a frame of lag would show as the horizon
                        // sliding back into place after every pan.
                        Sky::follow_lens,
                        Actors::place_labels,
                        EventLog::follow_playhead,
                        Timeline::refresh,
                        // After `ChunkLoader::pump`, which is where the spans
                        // arrive — the bar is drawn on the frame the recording
                        // tells us where its holes are.
                        Timeline::refresh_gaps,
                        Timeline::refresh_camera_reset,
                        Timeline::refresh_speed,
                        // After `handle_touch`, so the knob is drawn where the
                        // thumb has just put it rather than where it was.
                        FlightPad::refresh,
                        // Last of the bar's systems: the two above decide
                        // which buttons are lit, and these turn that plus the
                        // pointer into the colours actually drawn.
                        Timeline::paint_chips,
                        Timeline::paint_play,
                        // After the bar, because it is the last thing that
                        // reads a settled frame and the first that would
                        // interrupt one: a tier that steps down here does it
                        // between two frames rather than in the middle of
                        // drawing one. See `quality`, which explains why this
                        // may fire exactly once.
                        Quality::relent,
                        // Behind the whole frame, because the dip is drawn over
                        // all of it: whatever the cut this frame did to the
                        // camera, the bodies and the plates, this is what the
                        // viewer sees it through. See `cut`.
                        CutFade::follow_playhead,
                    ),
                    // Outside the two groups above rather than at the end of
                    // one, because both are full — Bevy's system tuples stop at
                    // twenty. It has to stay LAST all the same: it clears the
                    // flags every one of them reads.
                    Playback::end_frame,
                    FrameCost::leave_update,
                )
                    .chain(),
            )
            // Either end of the frame itself. `First` is ahead of everything
            // Bevy runs on this thread and `Last` is behind it, so what falls
            // outside the pair is the render sub-app and the browser — which
            // is the split that says whether a slow frame is ours or the GPU's.
            .add_systems(First, FrameCost::open)
            .add_systems(Last, FrameCost::close)
            // The corner frame counter. Registered apart from the full chain
            // above — which is at Bevy's twenty-system tuple limit — and
            // deliberately unordered against it: the badge paces itself and
            // reads a median that is always a frame stale anyway.
            .add_systems(Update, Timeline::refresh_fps)
            // The engine-facing overlays only exist when the page asked for
            // them, so their systems are only registered then.
            .add_systems(
                Update,
                (
                    Timeline::handle_debug_controls,
                    Actors::follow_states,
                    Timeline::refresh_debug,
                )
                    .chain()
                    .run_if(move || debug),
            )
            .run();
    }
}
