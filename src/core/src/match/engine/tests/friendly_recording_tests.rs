//! A friendly is recorded on the same flag as everything else.
//!
//! This existed as three independent `!friendly` exclusions — one in
//! [`Match::play`], one at the store in `web/game/process.rs`, one on the
//! match page — dating from when a recording meant a full ninety minutes of
//! samples and there were six times as many youth fixtures as senior ones to
//! pay for. `League::friendly` is what marks a youth or reserve competition,
//! so "friendly" here means every U18/U19/U20/U21/U23 and reserve league
//! match, not just a pre-season kickabout.
//!
//! Recordings are the goals and nothing else by default
//! (`RecordingScope::Goals`), which is what removed the reason for the
//! exclusion — and it is worth a test because the failure is silent. Re-add
//! any one of those three conditions and the game still builds, still plays,
//! still writes senior recordings, and every reserve-team replay in it is
//! quietly gone.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::MatchRuntime;
use crate::r#match::Match;
use crate::r#match::engine::engine::MATCH_TIME_MS;

/// Play one friendly and report whether the recording came out with anything
/// in it, and how far into the match it reaches.
fn friendly_recording(id: &str, base: u32) -> (bool, u64) {
    let result = Match::make(
        id.to_string(),
        100_000,
        "premier-league-u19",
        squad(1, base),
        squad(2, base + 100),
        true,
    )
    .play();

    let data = &result
        .details
        .as_ref()
        .expect("a played match has details")
        .position_data;
    (!data.is_empty(), data.max_timestamp())
}

/// One test rather than two, for the same reason
/// `goal_clip_recording_tests` gives: `recordings_mode` is process-global and
/// libtest runs test functions concurrently, so a second test that played a
/// match would read whichever value this one happened to have set. Written as
/// two tests first, and it failed exactly that way — the off-case switched the
/// flag off under the on-case mid-play.
#[test]
fn a_friendly_is_recorded_on_the_same_flag_as_a_league_match() {
    let previous = MatchRuntime::recordings_mode();

    // ── With recordings on, a youth-league match must produce one.
    //
    // Asserted on the track rather than on the flag, because the flag being
    // right and the recorder never being reached is exactly the shape of the
    // bug this guards.
    MatchRuntime::set_recordings_mode(true);
    let (recorded, reach_ms) = friendly_recording("friendly-on", 100);
    assert!(
        recorded,
        "a friendly played with recordings enabled produced no track at all"
    );
    // How MUCH of the match it keeps is `goal_clip_recording_tests`' business;
    // all this needs is that the recorder was reached and kept running to the
    // end. Measured against `MATCH_TIME_MS` rather than a literal, because a
    // debug build plays 5-minute halves and a release build 45 — the first
    // draft of this hardcoded ten minutes and failed on the exact number that
    // proves it worked.
    assert!(
        reach_ms >= MATCH_TIME_MS - 60_000,
        "the friendly's recording stops at {reach_ms} ms of a {MATCH_TIME_MS} ms match"
    );

    // ── …and the flag still turns it off.
    //
    // The other half of the contract: `--match-recording-disabled` has to mean
    // it for a friendly too, or the opt-out silently stops covering five
    // sixths of the fixtures in the game.
    MatchRuntime::set_recordings_mode(false);
    let (recorded, reach_ms) = friendly_recording("friendly-off", 300);
    assert!(
        !recorded,
        "recordings are off, yet the friendly still wrote a track reaching {reach_ms} ms"
    );

    MatchRuntime::set_recordings_mode(previous);
}
