//! How the match is shown: where the camera is, what it is pointed at, and the
//! stretches where the shot is written rather than followed.
//!
//! [`camera`] is the rig and every control on it — the broadcast follow, the
//! orbit, the zoom and the free flight. [`focus`] is which of the twenty-two
//! it has been asked to watch. [`cut`] is the dip the picture comes up through
//! when playback jumps a hole in the recording. [`changeover`], [`lineup`] and
//! [`goal`] are the three moments with a shot of their own: a substitution,
//! the two elevens before the first whistle, and the ball hitting the net.

use bevy::prelude::*;

pub(crate) mod camera;
pub(crate) mod changeover;
pub(crate) mod cut;
pub(crate) mod focus;
pub(crate) mod goal;
pub(crate) mod lineup;

/// How far a written shot has taken the picture off the broadcast rig, 0..1,
/// and the one rule every one of them moves it by.
///
/// Each shot in here keeps its own `grip` and each walks it at its own speed,
/// but they all walk it the same way: a fixed number of seconds end to end,
/// both directions, whatever the frame rate. Written once here rather than
/// three times because the three have to AGREE — a shot that eased where its
/// neighbour stepped would hand over at a different rate than it took over,
/// and the join is the only part of a camera move anybody sees.
pub struct Grip;

impl Grip {
    /// One frame of a ramp from `from` towards `to`, `seconds` long end to end.
    pub fn toward(from: f32, to: f32, seconds: f32, time: &Time) -> f32 {
        if from == to {
            return to;
        }
        let step = time.delta_secs() / seconds;
        if from < to {
            (from + step).min(to)
        } else {
            (from - step).max(to)
        }
    }
}
