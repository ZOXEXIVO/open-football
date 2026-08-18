//! Serialises the tests that drive the process-global recording switches.
//!
//! `MatchRuntime`'s recording state — the on/off flag and the
//! [`RecordingScope`] — is process-global, and the engine reads the scope
//! once, when it builds the recorder at kickoff (see
//! `engine::engine::run`). Both recording tests already knew that much:
//! each is written as one test function rather than two precisely because
//! libtest runs test functions concurrently and a sibling would read
//! whichever value the other had left behind.
//!
//! What neither guarded against was the *other file*. They set different
//! switches, so nothing looked like a conflict — but they read the same
//! recorder. `goal_clip_recording_tests` holds `RecordingScope::Goals` for
//! as long as it takes to play up to five matches, and any friendly that
//! kicks off inside that window is clipped to its goals; a goalless one
//! then produces no track at all, which is the flake this fixes.
//!
//! One lock across both files, taken for the whole span in which a test
//! either writes a recording switch or plays a match whose recording it
//! then asserts on.
//!
//! [`RecordingScope`]: crate::r#match::RecordingScope

#![cfg(test)]

use std::sync::{Mutex, MutexGuard};

static RECORDING_GLOBALS: Mutex<()> = Mutex::new(());

pub struct RecordingGlobals;

impl RecordingGlobals {
    /// Take exclusive ownership of the recording switches for the rest of
    /// the caller's scope.
    ///
    /// Poisoning is deliberately ignored: a panicking test leaves the
    /// switches in whatever state it got to, but every caller pins the
    /// values it depends on after taking the lock, so the next test in
    /// line is unaffected — and failing them all on the first failure
    /// would only hide which one actually broke.
    pub fn lock() -> MutexGuard<'static, ()> {
        RECORDING_GLOBALS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
