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
mod camera;
mod config;
mod field;
mod kit;
mod loader;
mod net;
mod pitch;
mod playback;
mod portrait;
mod replay;
mod sky;
mod textures;
mod timeline;
mod touch;
mod typeface;

use crate::actors::{Actors, BallState};
use crate::aftermath::Aftermath;
use crate::camera::{CameraFlight, CameraOrbit, CameraZoom, TvCamera};
use crate::config::ViewerConfig;
use crate::loader::ChunkLoader;
use crate::net::Netting;
use crate::pitch::{Bank, Pitch};
use crate::playback::{EventLog, Playback, RecordedSpans};
use crate::portrait::Portraits;
use crate::replay::ReplayTracks;
use crate::sky::Sky;
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
            .insert_resource(config)
            .insert_resource(Playback::new(duration_ms))
            .init_resource::<ReplayTracks>()
            .init_resource::<ChunkLoader>()
            .init_resource::<RecordedSpans>()
            .init_resource::<BallState>()
            .init_resource::<Aftermath>()
            .init_resource::<DebugOverlay>()
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
            // The touch half of the same controls. `TouchDrive` is read by
            // `CameraFlight::steer` on every frame whether or not anything has
            // ever been touched, so it is registered unconditionally — the
            // orbit's own missing registration is the cautionary tale a few
            // lines up.
            .init_resource::<TouchDevice>()
            .init_resource::<TouchDrive>()
            .init_resource::<TouchGesture>()
            .init_resource::<FlightPad>()
            .add_systems(
                Startup,
                (
                    Pitch::spawn,
                    Sky::spawn,
                    TvCamera::spawn,
                    Actors::spawn,
                    Timeline::spawn,
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
            .add_systems(
                Update,
                // Split into two nested groups purely to stay inside
                // Bevy's maximum tuple arity — `.chain()` still orders the
                // whole thing end to end, so the ordering notes below mean
                // exactly what they say across the boundary.
                (
                    (
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
                        Playback::handle_keyboard,
                        Playback::advance,
                        Actors::follow_playhead,
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
                        Playback::end_frame,
                    ),
                )
                    .chain(),
            )
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
