//! A national-team match is recorded on the same flag as everything else.
//!
//! An international is the one fixture in the game that never becomes a
//! [`Match`]. The world orchestrator builds two squads out of players scattered
//! across every continent and hands them straight to
//! [`MatchPlayEnginePool::play_squads_with_knockout`] — the only engine entry
//! point that does not go through [`Match::play`], and therefore the only one
//! that never read `MatchRuntime::recordings_mode()`. It passed a literal
//! `false` from the day it was written.
//!
//! So every international ever played, senior and U21 alike, was played with
//! the recorder switched off. The fixture appeared on the schedule, the match
//! page rendered both XIs with their ratings, and the replay had nothing to
//! offer but "Nothing was recorded in this match" — reported twice as "I cannot
//! see U21 matches in UI".
//!
//! Worth a test for the same reason `friendly_recording_tests` is: the failure
//! is silent. Put the literal back and the game still builds, still plays,
//! still writes every club recording in the world.
//!
//! [`Match`]: crate::r#match::Match
//! [`Match::play`]: crate::r#match::Match::play

#![cfg(test)]

use super::goal_celebration_tests::squad;
use super::recording_globals::RecordingGlobals;
use crate::MatchRuntime;
use crate::r#match::engine::engine::MATCH_TIME_MS;
use crate::r#match::{MatchPlayEnginePool, RecordingScope};

/// Play one squad-vs-squad fixture down the national-team path and report
/// whether a track came out of it, and how far into the match it reaches.
///
/// Deliberately routed through the pool rather than `FootballEngine::play`
/// directly: the pool is where the flag was being dropped, so calling the
/// engine would assert the recorder works and prove nothing about the path
/// an international actually takes.
fn international_recording(base: u32) -> (bool, u64) {
    let pool = MatchPlayEnginePool::new(1);
    let mut results = pool.play_squads(vec![(0, squad(791, base), squad(799, base + 100))]);
    let (_, raw) = results.pop().expect("one fixture in, one result out");
    (
        !raw.position_data.is_empty(),
        raw.position_data.max_timestamp(),
    )
}

/// One test rather than two: `recordings_mode` is process-global and libtest
/// runs test functions concurrently, so a second test that played a match
/// would read whichever value this one happened to have set. The shared lock
/// keeps the other recording files out of the window as well — see
/// [`RecordingGlobals`].
#[test]
fn an_international_is_recorded_on_the_same_flag_as_a_club_match() {
    let _globals = RecordingGlobals::lock();

    let previous_mode = MatchRuntime::recordings_mode();
    let previous_scope = MatchRuntime::recording_scope();

    // What this asserts is that the recorder was reached at all and ran to the
    // whistle. How much a *clipped* recording keeps is
    // `goal_clip_recording_tests`' business, and a goalless fixture clipped to
    // its goals is an empty track — which reads here exactly like the bug.
    MatchRuntime::set_recording_scope(RecordingScope::Full);

    // ── With recordings on, an international must produce one.
    //
    // Asserted on the track rather than on the flag: the flag being right and
    // the recorder never being reached is precisely the shape of this bug.
    MatchRuntime::set_recordings_mode(true);
    let (recorded, reach_ms) = international_recording(9100);
    assert!(
        recorded,
        "an international played with recordings enabled produced no track at all"
    );
    // Measured against `MATCH_TIME_MS`, not a literal: a debug build plays
    // 5-minute halves and a release build 45.
    assert!(
        reach_ms >= MATCH_TIME_MS - 60_000,
        "the international's recording stops at {reach_ms} ms of a {MATCH_TIME_MS} ms match"
    );

    // ── …and the flag still turns it off.
    //
    // The other half of the contract: `--match-recording-disabled` has to mean
    // it for a national-team fixture too, or the opt-out silently stops
    // covering the one match kind that reaches this path.
    MatchRuntime::set_recordings_mode(false);
    let (recorded, reach_ms) = international_recording(9300);
    assert!(
        !recorded,
        "recordings are off, yet the international still wrote a track reaching {reach_ms} ms"
    );

    MatchRuntime::set_recordings_mode(previous_mode);
    MatchRuntime::set_recording_scope(previous_scope);
}
