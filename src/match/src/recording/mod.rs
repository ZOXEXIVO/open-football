//! What the `core` crate wrote down, and the playhead that runs through it.
//!
//! [`replay`] is the shape of the recording — samples, the tracks they are
//! read out of and the events hung off them. [`loader`] fetches it a chunk at
//! a time, so the replay starts before it has all arrived. [`playback`] owns
//! the clock: every position, pose, camera move and label in the crate is
//! drawn from the time it holds.

pub(crate) mod loader;
pub(crate) mod playback;
pub(crate) mod replay;
