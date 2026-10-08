//! The match clock: stoppage time as the referee's allowance for the dead
//! time a period actually contained, and the whole-match readings the
//! players act on.

#![cfg(test)]

use super::goal_celebration_tests::squad;
use crate::r#match::engine::ball::ball::{AwaitedRestart, PassOriginRestart, PhaseOrigin};
use crate::r#match::engine::context::MatchEngineConfig;
use crate::r#match::engine::engine::FootballEngine;
use crate::r#match::engine::result::{DeadTime, PlayingTime, Score};
use crate::r#match::{
    MATCH_EXTRA_TIME_MS, MATCH_HALF_TIME_MS, MATCH_TIME_MS, MatchContext, MatchField,
    MatchPlayerCollection, MatchState, StateManager, SubstitutionWindows,
};
use nalgebra::Vector3;

/// 30 s of a substitution being played out, in 10 ms ticks.
const SUBSTITUTION_TICKS: u32 = 3_000;
/// 60 s of a player being treated.
const TREATMENT_TICKS: u32 = 6_000;

fn second_half() -> (MatchField, MatchContext) {
    let home = squad(1, 100);
    let away = squad(2, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(1, 2), false, false);
    context.state.set(MatchState::SecondHalf);
    (field, context)
}

fn play(context: &mut MatchContext, time: PlayingTime, ticks: u32) {
    for _ in 0..ticks {
        context.note_tick(time);
    }
}

#[test]
fn a_busier_period_ends_later() {
    let (_, mut quiet) = second_half();
    let (_, mut busy) = second_half();

    play(&mut quiet, PlayingTime::Live, 20_000);
    play(&mut busy, PlayingTime::Live, 20_000);
    play(
        &mut busy,
        PlayingTime::Dead(DeadTime::Substitution),
        SUBSTITUTION_TICKS,
    );
    play(
        &mut busy,
        PlayingTime::Dead(DeadTime::Substitution),
        SUBSTITUTION_TICKS,
    );
    play(
        &mut busy,
        PlayingTime::Dead(DeadTime::Treatment),
        TREATMENT_TICKS,
    );

    assert_eq!(quiet.period_stoppage_time_ms, 0);
    // Two changes and a treatment are two minutes of dead ball; the
    // referee gives most of it back.
    assert!(
        busy.period_stoppage_time_ms > 90_000 && busy.period_stoppage_time_ms <= 120_000,
        "got {} ms",
        busy.period_stoppage_time_ms
    );
}

#[test]
fn waiting_for_a_throw_in_is_not_added_back() {
    let (_, mut context) = second_half();
    play(&mut context, PlayingTime::Dead(DeadTime::Restart), 5_000);
    assert_eq!(context.period_stoppage_time_ms, 0);
    assert_eq!(context.tally.dead_ms[DeadTime::Restart.index()], 50_000);
}

#[test]
fn dead_time_outside_a_period_changes_nothing() {
    let (_, mut context) = second_half();
    context.state.set(MatchState::HalfTime);
    play(
        &mut context,
        PlayingTime::Dead(DeadTime::Celebration),
        6_000,
    );
    assert_eq!(context.period_stoppage_time_ms, 0);
}

#[test]
fn a_booking_holds_the_restart_and_reads_as_booking_time() {
    let (mut field, context) = second_half();
    let now = context.current_tick();
    let mut restart = awaited(PassOriginRestart::DirectFreeKick, now);
    restart.hold_until(DeadTime::Booking, now + 2_000);
    field.ball.awaiting_restart = Some(restart);

    assert_eq!(
        field.ball.playing_time(now + 10),
        PlayingTime::Dead(DeadTime::Booking)
    );
    assert_eq!(
        field.ball.playing_time(now + 2_000),
        PlayingTime::Dead(DeadTime::Restart)
    );

    // A shorter second hold does not cut the first one short.
    restart.hold_until(DeadTime::Delay, now + 500);
    assert!(restart.is_held(now + 1_500));
}

#[test]
fn the_last_third_is_read_off_the_whole_match() {
    let (_, mut context) = second_half();
    // Well into the second half: the period clock has barely passed its
    // midpoint, the match is nearly three quarters done.
    context.time.time = MATCH_HALF_TIME_MS * 2 / 5;
    context.total_match_time = MATCH_TIME_MS * 7 / 10;
    assert!(context.is_running_out());

    context.total_match_time = MATCH_TIME_MS * 11 / 20;
    assert!(!context.is_running_out());
}

#[test]
fn extra_time_is_two_periods_with_a_change_of_ends_between_them() {
    let (mut field, mut context) = second_half();
    context.is_knockout = true;
    let (home, away) = (field.home_team_id, field.away_team_id);

    StateManager::kick_off_period(&mut context, &mut field);
    assert_eq!(context.period_kickoff_team, Some(home));

    context.state.set(MatchState::HalfTime);
    StateManager::handle_state_finish(&mut context, &mut field);
    assert_eq!(context.period_kickoff_team, Some(away));
    let second_half_end = field.side_of(home);

    // Level after ninety: extra time, kicked off by the side that did not
    // kick off the second half, with one more change and one more stoppage.
    let changes = context.max_substitutions_per_team;
    context.state.set(MatchState::SecondHalf);
    StateManager::handle_state_finish(&mut context, &mut field);
    assert_eq!(context.period_kickoff_team, Some(home));
    assert_eq!(context.max_substitutions_per_team, changes + 1);
    assert_eq!(
        context.substitution_windows.allowance(),
        SubstitutionWindows::PER_TEAM + 1
    );
    assert_eq!(field.side_of(home), second_half_end);

    context.state.set(MatchState::ExtraTimeFirst);
    StateManager::handle_state_finish(&mut context, &mut field);
    context.state.set(MatchState::ExtraTimeInterval);
    StateManager::handle_state_finish(&mut context, &mut field);
    assert_ne!(field.side_of(home), second_half_end);
    assert_eq!(context.period_kickoff_team, Some(away));
}

#[test]
fn a_level_knockout_plays_both_periods_of_extra_time() {
    for seed in 0..20u64 {
        let config = MatchEngineConfig {
            is_knockout: true,
            ..MatchEngineConfig::seeded(0x0E7_0000 + seed)
        };
        let result =
            FootballEngine::<840, 545>::play_with_config(squad(1, 100), squad(2, 200), config);
        let reached_extra_time = !result.penalty_shootout.is_empty()
            || result.match_time_ms
                > MATCH_TIME_MS
                    + result.tally.added_time_ms[0]
                    + result.tally.added_time_ms[1]
                    + 60_000;
        if reached_extra_time {
            assert!(
                result.match_time_ms >= MATCH_TIME_MS + 2 * MATCH_EXTRA_TIME_MS,
                "extra time ran {} ms of a {} ms pair of periods",
                result.match_time_ms.saturating_sub(MATCH_TIME_MS),
                2 * MATCH_EXTRA_TIME_MS
            );
            return;
        }
    }
    panic!("twenty knockout matches and none went to extra time");
}

fn awaited(origin: PassOriginRestart, tick: u64) -> AwaitedRestart {
    AwaitedRestart {
        taker_id: 109,
        spot: Vector3::new(746.0, 272.0, 0.0),
        take_from: None,
        settled: true,
        carrying: false,
        origin,
        awarded_tick: tick,
        patience_ticks: AwaitedRestart::PATIENCE_TICKS,
        settled_tick: None,
        hold: None,
    }
}

#[test]
fn a_penalty_holds_the_period_open_until_it_is_finished() {
    let (mut field, context) = second_half();
    let now = context.current_tick();

    field.ball.awaiting_restart = Some(awaited(PassOriginRestart::Penalty, now));
    assert!(field.ball.penalty_in_progress(now), "awarded and not taken");

    // Taken: the kick is the taker's only touch, and the ball is travelling.
    field.ball.awaiting_restart = None;
    let taker = field
        .players
        .iter()
        .find(|p| p.team_id == field.home_team_id)
        .unwrap()
        .id;
    let keeper = field
        .players
        .iter()
        .find(|p| p.team_id == field.away_team_id)
        .unwrap()
        .id;
    let mut phase = PhaseOrigin::open(PassOriginRestart::Penalty, field.home_team_id, taker, now);
    phase.note_touch(taker, field.home_team_id, now, true);
    field.ball.phase_origin = Some(phase);
    assert!(
        field.ball.penalty_in_progress(now + 30),
        "struck, still travelling"
    );

    // The keeper gets a hand to it: the kick is over.
    let mut saved = phase;
    saved.note_touch(keeper, field.away_team_id, now + 40, false);
    field.ball.phase_origin = Some(saved);
    assert!(!field.ball.penalty_in_progress(now + 40));

    // A ball nobody touches again cannot hold the period open for ever.
    field.ball.phase_origin = Some(phase);
    assert!(
        !field
            .ball
            .penalty_in_progress(now + PhaseOrigin::MAX_TICKS + 1)
    );

    // Wide: a goal kick is awaited, and the penalty is over.
    field.ball.awaiting_restart = Some(awaited(PassOriginRestart::GoalKick, now + 60));
    assert!(!field.ball.penalty_in_progress(now + 60));
}
