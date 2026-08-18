//! Goal-clip recording, over a real match.
//!
//! `result.rs` has unit tests for the clipping itself — what a clip holds,
//! how two of them merge, what a goalless match reports. What it cannot test
//! is the one line that makes any of it happen: the engine telling the
//! recorder that a goal has gone in, at the instant it went in, from inside
//! the tick.
//!
//! That wiring fails silently. Mark the goal a tick too late and the flag has
//! already been cleared; mark it off the wrong clock and every clip lands in
//! the wrong minute; miss one of the two tick paths and half the goals in the
//! game go unrecorded. In each case the unit tests still pass and every
//! recording the game writes is empty or wrong — which is why this is worth a
//! test that plays ninety minutes, the only thing that reaches those lines.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use super::recording_globals::RecordingGlobals;
use crate::MatchRuntime;
use crate::r#match::MatchResultRaw;
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::player::statistics::MatchStatisticType;
use crate::r#match::result::{GOAL_CLIP_POST_ROLL_MS, GOAL_CLIP_PRE_ROLL_MS, RecordingScope};

/// Plays until a match with at least one goal comes out, and returns it.
///
/// Nothing is asserted about WHICH match: a goalless one is a legitimate
/// result this file has nothing to say about, and pinning a seed would only
/// make the test brittle — the engine's stream is perturbed by whatever else
/// the suite is running (see the `core_suite_rng_flakiness` note). At two and
/// a bit goals a match, five goalless in a row is a one-in-a-hundred-thousand
/// event — and each retry is a whole ninety minutes, so the ceiling is low on
/// purpose.
fn match_with_a_goal() -> MatchResultRaw {
    for seed in 0..5u64 {
        let mut config = MatchEngineConfig::seeded(0x0F00_0000 + seed);
        config.match_recordings = true;
        let result =
            FootballEngine::<840, 545>::play_with_config(squad(1, 100), squad(2, 200), config);
        let score = result.score.as_ref().expect("a score");
        if score.home_team.get() + score.away_team.get() > 0 {
            return result;
        }
    }
    panic!("five matches in a row finished goalless — the engine is not scoring at all");
}

/// One test rather than two, because the scope is process-global (like
/// `events_mode` beside it) and libtest runs test functions concurrently — a
/// second test that played a match would read whichever scope this one
/// happened to have set.
#[test]
fn a_clipped_recording_holds_the_goals_and_nothing_else() {
    // Held for as long as the scope is narrowed. `friendly_recording_tests`
    // plays matches whose recordings it asserts run end to end, and the
    // engine reads the scope at kickoff — without this it can kick off inside
    // the window below and come back clipped. See `recording_globals`.
    let _globals = RecordingGlobals::lock();

    // The default has to stay `Full`. The dev harness and every calibration
    // run read the whole match back off the recording and never touch the
    // scope; a default that clipped would gut them silently.
    assert_eq!(
        MatchRuntime::recording_scope(),
        RecordingScope::Full,
        "the process default must keep whole matches — only the game narrows it"
    );

    MatchRuntime::set_recording_scope(RecordingScope::Goals);
    let result = match_with_a_goal();
    MatchRuntime::set_recording_scope(RecordingScope::Full);

    let goals: Vec<u64> = result
        .score
        .as_ref()
        .expect("a score")
        .detail()
        .iter()
        .filter(|event| event.stat_type == MatchStatisticType::Goal)
        .map(|event| event.time)
        .collect();
    assert!(!goals.is_empty(), "the harness returned a goalless match");

    let data = &result.position_data;
    let segments = data
        .recorded_segments()
        .expect("a clipped recording must advertise its segments")
        .to_vec();

    assert!(
        !segments.is_empty(),
        "a match with {} goal(s) recorded no clips — the engine never told the \
         recorder a goal had gone in",
        goals.len()
    );
    assert!(
        segments.len() <= goals.len(),
        "more clips than goals: {segments:?} for goals at {goals:?}"
    );

    // Every goal falls inside a clip. This is what catches a mark taken off
    // the wrong clock: an off-by-a-period timestamp still produces
    // plausible-looking segments, just not ones with goals in them.
    for goal in &goals {
        assert!(
            segments
                .iter()
                .any(|(start, end)| goal >= start && goal <= end),
            "the goal at {goal} ms is not inside any clip: {segments:?}"
        );
    }

    // And a clip is the window it claims to be, not the whole match.
    let recorded: u64 = segments.iter().map(|(start, end)| end - start).sum();
    let budget = goals.len() as u64 * (GOAL_CLIP_PRE_ROLL_MS + GOAL_CLIP_POST_ROLL_MS);
    assert!(
        recorded <= budget,
        "kept {recorded} ms for {} goal(s), more than the {budget} ms the clips \
         are allowed: {segments:?}",
        goals.len()
    );
    assert!(
        recorded < result.match_time_ms,
        "the clipped recording covers the whole match"
    );

    // The samples themselves stop where the clips do — a segment list is a
    // claim about the recording, not a substitute for it.
    let last = data.max_timestamp();
    assert!(
        segments
            .iter()
            .any(|(start, end)| last >= *start && last <= *end),
        "the recording's last sample, at {last} ms, is outside every clip: {segments:?}"
    );
}
