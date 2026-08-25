//! Bevy/WebAssembly viewer for matches recorded by the `core` crate.
//!
//! The web crate serves the match page, hands this module a canvas and a JSON
//! description of the fixture, and gets out of the way: fetching the recorded
//! position chunks, rendering the pitch in 3D and driving playback all happen
//! here, so the replay is one artefact rather than a Rust half and a JavaScript
//! half that have to agree with each other.

mod actors;
mod aftermath;
mod body;
mod bringup;
mod camera;
mod changeover;
mod config;
mod cut;
mod field;
mod focus;
mod kit;
mod loader;
mod net;
mod perf;
mod pitch;
mod playback;
mod portrait;
mod quality;
mod replay;
mod sky;
mod stage;
mod textures;
mod timeline;
mod touch;
mod typeface;

use crate::actors::{Actors, BallState};
use crate::aftermath::Aftermath;
use crate::bringup::Bringup;
use crate::camera::{CameraFlight, CameraOrbit, CameraZoom, TvCamera};
use crate::changeover::ChangeoverShot;
use crate::config::ViewerConfig;
use crate::cut::CutFade;
use crate::focus::{CameraSubject, FocusRing};
use crate::loader::ChunkLoader;
use crate::net::Netting;
use crate::perf::FrameCost;
use crate::pitch::{Bank, Pitch};
use crate::playback::{EventLog, Playback, RecordedSpans};
use crate::portrait::Portraits;
use crate::quality::Quality;
use crate::replay::ReplayTracks;
use crate::sky::Sky;
use crate::stage::Stage;
use crate::timeline::{DebugOverlay, Timeline};
use crate::touch::{FlightPad, TouchControls, TouchDevice, TouchDrive, TouchGesture};
use crate::typeface::Typeface;
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

        App::new()
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
