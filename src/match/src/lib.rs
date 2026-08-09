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
use crate::camera::TvCamera;
use crate::config::ViewerConfig;
use crate::loader::ChunkLoader;
use crate::pitch::Pitch;
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
                    Playback::handle_keyboard,
                    Playback::advance,
                    Actors::follow_playhead,
                    Actors::animate,
                    TvCamera::follow_play,
                    Actors::place_labels,
                    EventLog::follow_playhead,
                    Timeline::refresh,
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
