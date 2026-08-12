//! Bevy/WebAssembly viewer for matches recorded by the `core` crate.
//!
//! The web crate serves the match page, hands this module a canvas and a JSON
//! description of the fixture, and gets out of the way: fetching the recorded
//! position chunks, rendering the pitch in 3D and driving playback all happen
//! here, so the replay is one artefact rather than a Rust half and a JavaScript
//! half that have to agree with each other.

mod actors;
mod body;
mod camera;
mod config;
mod field;
mod kit;
mod loader;
mod pitch;
mod playback;
mod replay;
mod textures;
mod timeline;

use crate::actors::{Actors, BallState};
use crate::camera::{CameraFlight, CameraOrbit, CameraZoom, TvCamera};
use crate::config::ViewerConfig;
use crate::loader::ChunkLoader;
use crate::pitch::{Bank, Pitch};
use crate::playback::{EventLog, Playback};
use crate::replay::ReplayTracks;
use crate::timeline::{DebugOverlay, Timeline};
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
                    // the only asset is the embedded default font — so asking
                    // for `.meta` sidecars would only produce 404s.
                    .set(AssetPlugin {
                        meta_check: AssetMetaCheck::Never,
                        ..default()
                    })
                    .set(LogPlugin {
                        level: Level::WARN,
                        ..default()
                    }),
            )
            .insert_resource(ClearColor(Color::srgb(0.03, 0.05, 0.07)))
            .insert_resource(config)
            .insert_resource(Playback::new(duration_ms))
            .init_resource::<ReplayTracks>()
            .init_resource::<ChunkLoader>()
            .init_resource::<BallState>()
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
            .add_systems(
                Startup,
                (
                    Pitch::spawn,
                    TvCamera::spawn,
                    Actors::spawn,
                    Timeline::spawn,
                    ChunkLoader::bootstrap,
                ),
            )
            .add_systems(
                Update,
                (
                    ChunkLoader::pump,
                    Timeline::handle_toggle,
                    Timeline::handle_seek,
                    // Ahead of every camera system below, so a click on the
                    // reset chip lands on the frame it happened.
                    Timeline::handle_camera_reset,
                    Playback::handle_keyboard,
                    Playback::advance,
                    Actors::follow_playhead,
                    Actors::animate,
                    // Ahead of `follow_play`, which reads the orbit — so a
                    // drag lands on the same frame it happened rather than
                    // the next one. Never registered at all before, so the
                    // camera could not be turned.
                    CameraOrbit::handle_drag,
                    CameraZoom::handle_wheel,
                    TvCamera::follow_play,
                    // After `follow_play` rather than before it: on the frame
                    // the rig takes off, flight seeds itself from the
                    // broadcast position that system has just written, so the
                    // first key press continues the shot instead of cutting.
                    CameraFlight::steer,
                    // Straight after the camera moves, so the stand the
                    // rig has just walked into is gone on the same frame
                    // it entered rather than flashing for one.
                    Bank::cull,
                    Actors::place_labels,
                    EventLog::follow_playhead,
                    Timeline::refresh,
                    Timeline::refresh_camera_reset,
                    Playback::end_frame,
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
