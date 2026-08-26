//! How the match is shown: where the camera is, what it is pointed at, and the
//! stretches where the shot is written rather than followed.
//!
//! [`camera`] is the rig and every control on it — the broadcast follow, the
//! orbit, the zoom and the free flight. [`focus`] is which of the twenty-two
//! it has been asked to watch. [`cut`] is the dip the picture comes up through
//! when playback jumps a hole in the recording. [`changeover`] and [`lineup`]
//! are the two ceremonies with a shot of their own: a substitution, and the
//! two elevens before the first whistle.

pub(crate) mod camera;
pub(crate) mod changeover;
pub(crate) mod cut;
pub(crate) mod focus;
pub(crate) mod lineup;
