use crate::app::config::{ViewerConfig, ViewerLabels};
use crate::recording::loader::ChunkLoader;
use crate::recording::replay::ReplayTracks;
use bevy::prelude::*;
use wasm_bindgen::JsValue;

/// Which parts of the match the recording actually covers.
///
/// The game keeps the goals and throws the rest away, so a replay is a handful
/// of ten-second clips scattered across ninety minutes rather than a continuous
/// film. Everything that has to know where the holes are reads them from here:
/// the timeline greys them, playback jumps them, and the loader does not ask
/// the server for chunks that were never written.
///
/// `partial == false` — the default until the metadata says otherwise — means
/// the whole match is there and none of the above applies.
#[derive(Resource, Default)]
pub struct RecordedSpans {
    spans: Vec<(f64, f64)>,
    partial: bool,
    /// Bumped whenever the spans change, so the timeline knows to redraw them
    /// without diffing a vector every frame.
    revision: u32,
}

impl RecordedSpans {
    pub fn set(&mut self, spans: Vec<(f64, f64)>) {
        self.spans = spans;
        self.partial = true;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn partial(&self) -> bool {
        self.partial
    }

    pub fn revision(&self) -> u32 {
        self.revision
    }

    pub fn spans(&self) -> &[(f64, f64)] {
        &self.spans
    }

    /// True when the recording has nothing in it at all — a goalless match.
    pub fn nothing_recorded(&self) -> bool {
        self.partial && self.spans.is_empty()
    }

    /// Is there a recording at `time_ms`? Always yes when the whole match was
    /// kept.
    pub fn covers(&self, time_ms: f64) -> bool {
        !self.partial
            || self
                .spans
                .iter()
                .any(|(start, end)| time_ms >= *start && time_ms <= *end)
    }

    /// Does any span fall inside `[start, end)`? Used to decide whether a
    /// chunk is worth requesting.
    pub fn intersects(&self, start: f64, end: f64) -> bool {
        !self.partial
            || self
                .spans
                .iter()
                .any(|(from, to)| *from < end && *to >= start)
    }

    /// Start of the first clip that begins after `time_ms`.
    pub fn next_start(&self, time_ms: f64) -> Option<f64> {
        self.spans
            .iter()
            .map(|(start, _)| *start)
            .find(|start| *start > time_ms)
    }

    /// The nearest instant that actually has a recording behind it. Scrubbing
    /// into a hole lands on the edge of the clip either side of it rather than
    /// on a blank pitch.
    pub fn snap(&self, time_ms: f64) -> f64 {
        if self.covers(time_ms) || self.spans.is_empty() {
            return time_ms;
        }
        self.spans
            .iter()
            .flat_map(|(start, end)| [*start, *end])
            .min_by(|a, b| {
                (a - time_ms)
                    .abs()
                    .partial_cmp(&(b - time_ms).abs())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .unwrap_or(time_ms)
    }
}

/// The playhead. Replay time is wall-clock time — one second of viewing is one
/// second of the recording — so the only state here is where we are and whether
/// we are moving.
#[derive(Resource)]
pub struct Playback {
    pub time_ms: f64,
    pub duration_ms: f64,
    pub playing: bool,
    /// Multiplier on real time. Only the debug overlay offers a way to change
    /// it — reviewing an engine change means skimming a whole match.
    pub speed: f32,
    /// Set for one frame after a seek so followers can cut instead of glide.
    pub seeked: bool,
    /// Set for one frame when the playhead ran out of a clip and was carried
    /// to the next one — a cut the REPLAY made, as against a seek somebody
    /// asked for.
    ///
    /// The two are deliberately separate flags even though a cut is also a
    /// seek. A scrub sets `seeked` on every frame the pointer is held down, and
    /// [`crate::broadcast::cut`] hangs a half-second dip off this one: hung off
    /// `seeked` instead, dragging the knob across the rail would hold the
    /// picture dark for as long as the drag lasted, which is precisely when
    /// somebody needs to see where they are going.
    pub cut: bool,
}

impl Playback {
    /// Speeds the debug overlay cycles through, slowest first.
    ///
    /// The slow half is the half that earns its keep: reading whether a
    /// midfielder shoots or squares it, or whether a marker is goal-side,
    /// is not something you can see at real time, let alone at 16x. The
    /// fast half is for skimming a whole match.
    ///
    /// 1x is the DEFAULT (see `new`) — the cycle merely starts wherever
    /// the playhead currently is.
    pub const SPEEDS: [f32; 7] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];

    pub fn new(duration_ms: f64) -> Self {
        Playback {
            time_ms: 0.0,
            duration_ms,
            playing: false,
            speed: 1.0,
            seeked: false,
            cut: false,
        }
    }

    /// Steps to the next speed, wrapping from the fastest back to the
    /// slowest. An unrecognised speed lands on 1x rather than on the top
    /// of the list, so the control always has a way home to real time.
    pub fn cycle_speed(&mut self) {
        let next = Self::SPEEDS
            .iter()
            .position(|candidate| (*candidate - self.speed).abs() < f32::EPSILON)
            .map(|index| (index + 1) % Self::SPEEDS.len())
            .unwrap_or(Self::DEFAULT_INDEX);
        self.speed = Self::SPEEDS[next];
    }

    /// Index of 1x in [`Self::SPEEDS`].
    const DEFAULT_INDEX: usize = 2;

    /// How the speed reads on the chip.
    ///
    /// `speed as u32` was fine for a list of whole numbers and rounds
    /// every slow speed to **"0x"** — 0.25 and 0.5 both truncate to zero.
    /// Whole speeds still print bare (`2x`, not `2.00x`).
    pub fn speed_label(&self) -> String {
        if (self.speed - self.speed.round()).abs() < 1e-3 {
            format!("{}x", self.speed.round() as u32)
        } else {
            format!("{}x", self.speed)
        }
    }

    pub fn progress(&self) -> f32 {
        if self.duration_ms <= 0.0 {
            return 0.0;
        }
        (self.time_ms / self.duration_ms).clamp(0.0, 1.0) as f32
    }

    pub fn seek_to(&mut self, fraction: f32) {
        self.time_ms = (fraction.clamp(0.0, 1.0) as f64) * self.duration_ms;
        self.seeked = true;
    }

    /// Match clock as the scoreboard would read it — minutes into the half.
    pub fn clock_label(&self, labels: &ViewerLabels) -> String {
        self.stamp(self.time_ms, labels)
    }

    pub fn stamp(&self, time_ms: f64, labels: &ViewerLabels) -> String {
        let half = self.duration_ms * 0.5;
        let (offset, half_label) = if time_ms < half {
            (time_ms, &labels.first_half)
        } else {
            (time_ms - half, &labels.second_half)
        };
        let seconds = (offset / 1000.0).max(0.0) as u64;
        format!("{}:{:02} ({})", seconds / 60, seconds % 60, half_label)
    }

    pub fn advance(
        mut playback: ResMut<Playback>,
        loader: Res<ChunkLoader>,
        spans: Res<RecordedSpans>,
        time: Res<Time>,
    ) {
        if !playback.playing || !loader.ready {
            return;
        }
        playback.time_ms += time.delta_secs_f64() * 1000.0 * playback.speed as f64;

        // Run out of the end of a clip and there is nothing ahead but empty
        // pitch, so cut to the next one. `seeked` because that is what it is
        // — followers have to cut with it rather than glide across forty
        // minutes of match in a frame. And `cut`, because a viewer has to be
        // told: see [`crate::broadcast::cut`], which fades the next episode in.
        if !spans.covers(playback.time_ms) {
            match spans.next_start(playback.time_ms) {
                Some(start) => {
                    playback.time_ms = start;
                    playback.seeked = true;
                    playback.cut = true;
                }
                None => playback.time_ms = playback.duration_ms,
            }
        }

        if playback.time_ms >= playback.duration_ms {
            playback.time_ms = playback.duration_ms;
            playback.playing = false;
        }
    }

    /// Runs last: everything that reacts to a seek has had its turn by now.
    pub fn end_frame(mut playback: ResMut<Playback>) {
        playback.seeked = false;
        playback.cut = false;
    }

    /// Space bar toggles playback — the one keyboard shortcut worth having.
    pub fn handle_keyboard(mut playback: ResMut<Playback>, keys: Res<ButtonInput<KeyCode>>) {
        if keys.just_pressed(KeyCode::Space) {
            let restart = playback.time_ms >= playback.duration_ms;
            if restart {
                playback.seek_to(0.0);
            }
            playback.playing = !playback.playing || restart;
        }
    }
}

/// Mirrors the recorded engine events into the browser console as the playhead
/// passes them. This is the only view into the engine's own commentary, and the
/// pixi viewer had it — keep it.
pub struct EventLog;

impl EventLog {
    pub fn follow_playhead(
        playback: Res<Playback>,
        config: Res<ViewerConfig>,
        mut tracks: ResMut<ReplayTracks>,
    ) {
        if playback.seeked {
            let time = playback.time_ms;
            tracks.next_event = tracks
                .events
                .partition_point(|e| (e.timestamp as f64) <= time);
            return;
        }

        while tracks.next_event < tracks.events.len() {
            let event = &tracks.events[tracks.next_event];
            if (event.timestamp as f64) > playback.time_ms {
                break;
            }
            let (kind, color) = if event.category == "ball" {
                ("Ball", "color: #4fc3f7")
            } else {
                ("Player", "color: #81c784")
            };
            web_sys::console::log_2(
                &JsValue::from_str(&format!(
                    "%c[{}] {}: {}",
                    playback.stamp(event.timestamp as f64, &config.labels),
                    kind,
                    event.description
                )),
                &JsValue::from_str(color),
            );
            tracks.next_event += 1;
        }
    }
}
