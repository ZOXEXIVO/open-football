//! Who is charged with an error leading to a shot or a goal.
//!
//! A giveaway is stamped on the man whose own execution failed, charged
//! once by the first shot of the attack it gave away, promoted if that
//! attack scores, and closed when his side has the ball again or play
//! stops.

#![cfg(test)]

use super::goal_celebration_tests::{score_at_the_home_end, squad};
use crate::r#match::engine::ball::events::{BallEvent, BallEventDispatcher};
use crate::r#match::engine::player::events::players::PlayerEventDispatcher;
use crate::r#match::engine::psychology::{ActionLapse, LapseFamily, PsychState};
use crate::r#match::engine::result::Score;
use crate::r#match::player::events::PlayerEvent;
use crate::r#match::events::EventCollection;
use crate::r#match::{
    MatchContext, MatchField, MatchPlayerCollection, PlayerSide, ResultMatchPositionData,
};
use nalgebra::Vector3;

const KICKOFF_MS: u64 = 10 * 60 * 1000;
const HOME: u32 = 1;
const AWAY: u32 = 2;
/// A home defender.
const GIVER: u32 = 102;
/// A home midfielder.
const TEAMMATE: u32 = 106;
/// The away forwards.
const STRIKER: u32 = 209;
const RUNNER: u32 = 210;

fn kickoff() -> (MatchField, MatchContext) {
    let home = squad(HOME, 100);
    let away = squad(AWAY, 200);
    let players = MatchPlayerCollection::from_squads(&home, &away);
    let field = MatchField::new(840, 545, home, away);
    let mut context = MatchContext::new(&field, players, Score::new(HOME, AWAY), false, false);
    context.total_match_time = KICKOFF_MS;
    (field, context)
}

fn shot_by(shooter: u32, field: &mut MatchField, context: &mut MatchContext) {
    let team = field.get_player(shooter).map(|p| p.team_id);
    let tick = context.current_tick();
    PlayerEventDispatcher::charge_giveaway_on_shot(team, tick, field, context);
}

fn goal_by(scorer: u32, field: &mut MatchField, context: &mut MatchContext) {
    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(PlayerEvent::Goal(scorer, false), field, context, &mut data);
}

/// `(errors leading to a shot, errors leading to a goal)`.
fn errors(field: &MatchField, id: u32) -> (u16, u16) {
    let s = &field.get_player(id).unwrap().statistics;
    (s.errors_leading_to_shot, s.errors_leading_to_goal)
}

#[test]
fn a_rebound_is_charged_once_and_promoted() {
    let (mut field, mut context) = kickoff();
    PlayerEventDispatcher::lose_ball(GIVER, LapseFamily::Pass, &mut field, &context);

    shot_by(STRIKER, &mut field, &mut context);
    // Saved, and the rebound falls to the runner.
    shot_by(RUNNER, &mut field, &mut context);
    goal_by(RUNNER, &mut field, &mut context);

    assert_eq!(errors(&field, GIVER), (1, 1));
}

/// The netting drops everything a live ball carries except what the goal
/// handler has still to read — the charge among it.
#[test]
fn the_charge_survives_the_ball_going_into_the_net() {
    let (mut field, mut context) = kickoff();
    PlayerEventDispatcher::lose_ball(GIVER, LapseFamily::Pass, &mut field, &context);
    shot_by(STRIKER, &mut field, &mut context);
    let scorer = score_at_the_home_end(&mut field, &mut context);
    goal_by(scorer, &mut field, &mut context);

    assert_eq!(errors(&field, GIVER), (1, 1));
}

#[test]
fn a_misplaced_pass_collected_loose_is_an_error() {
    let (mut field, mut context) = kickoff();
    field.ball.pending_pass_passer = Some(GIVER);
    // Arriving 4 m off its ideal point.
    field.ball.pending_pass_error = 32.0;
    PlayerEventDispatcher::resolve_pending_pass_on_control(STRIKER, &mut field, &context);

    shot_by(STRIKER, &mut field, &mut context);

    assert_eq!(errors(&field, GIVER), (1, 0));
}

#[test]
fn a_pass_read_and_cut_out_is_not_an_error() {
    let (mut field, mut context) = kickoff();
    field.ball.pending_pass_passer = Some(GIVER);
    // Going where it was meant to, a foot off.
    field.ball.pending_pass_error = 8.0;
    PlayerEventDispatcher::resolve_pending_pass_on_control(STRIKER, &mut field, &context);

    shot_by(STRIKER, &mut field, &mut context);

    assert_eq!(field.ball.last_giveaway_player_id, None);
    assert_eq!(errors(&field, GIVER), (0, 0));
}

#[test]
fn a_pass_a_teammate_collects_is_not_a_giveaway() {
    let (mut field, context) = kickoff();
    field.ball.pending_pass_passer = Some(GIVER);
    PlayerEventDispatcher::resolve_pending_pass_on_control(TEAMMATE, &mut field, &context);

    assert_eq!(field.ball.last_giveaway_player_id, None);
}

#[test]
fn a_clean_tackle_is_not_an_error() {
    let (mut field, mut context) = kickoff();
    // Nothing of his failed: the ball was won off him, nothing stamped.
    shot_by(STRIKER, &mut field, &mut context);
    goal_by(STRIKER, &mut field, &mut context);

    assert_eq!(errors(&field, GIVER), (0, 0));
    assert_eq!(errors(&field, TEAMMATE), (0, 0));
}

#[test]
fn regaining_the_ball_clears_the_charge() {
    let (mut field, mut context) = kickoff();
    PlayerEventDispatcher::lose_ball(GIVER, LapseFamily::Pass, &mut field, &context);
    // His side wins it back, then loses it to a clean tackle inside the
    // original window.
    let tick = context.current_tick();
    field.ball.record_touch(TEAMMATE, HOME, tick, true);
    shot_by(STRIKER, &mut field, &mut context);

    assert_eq!(errors(&field, GIVER), (0, 0));
}

#[test]
fn a_dead_ball_ends_the_attack() {
    let (mut field, mut context) = kickoff();
    PlayerEventDispatcher::lose_ball(GIVER, LapseFamily::Pass, &mut field, &context);
    shot_by(STRIKER, &mut field, &mut context);
    // Blocked behind for a corner; the goal comes from the set piece.
    field.ball.clear_open_play_metadata();
    goal_by(RUNNER, &mut field, &mut context);

    assert_eq!(errors(&field, GIVER), (1, 0));
}

#[test]
fn an_uncontrolled_touch_by_his_side_does_not_end_it() {
    let (mut field, mut context) = kickoff();
    PlayerEventDispatcher::lose_ball(GIVER, LapseFamily::Touch, &mut field, &context);
    shot_by(STRIKER, &mut field, &mut context);
    // A team-mate's block, not possession.
    let tick = context.current_tick();
    field.ball.record_touch(TEAMMATE, HOME, tick, false);
    goal_by(RUNNER, &mut field, &mut context);

    assert_eq!(errors(&field, GIVER), (1, 1));
}

#[test]
fn a_failed_first_touch_books_one_touch() {
    let (mut field, mut context) = kickoff();
    let at = Vector3::new(300.0, 270.0, 0.0);
    for p in field.players.iter_mut() {
        if p.id == GIVER {
            p.position = at;
            let s = &mut p.skills;
            s.technical.first_touch = 1.0;
            s.technical.technique = 1.0;
            s.mental.composure = 1.0;
            s.mental.anticipation = 1.0;
            s.mental.decisions = 1.0;
        } else if p.team_id == AWAY && p.id % 100 >= 7 {
            // Four opponents on him: the stat-only roll would fire about
            // half the time on a reception like this.
            p.position = at + Vector3::new(6.0, (p.id % 100) as f32 - 8.5, 0.0);
        }
    }
    for minute_ms in 0..200u64 {
        context.total_match_time = KICKOFF_MS + minute_ms * 10;
        let mut out = EventCollection::new();
        BallEventDispatcher::dispatch(
            BallEvent::FirstTouchFailed(GIVER, TEAMMATE, minute_ms % 2 == 0),
            &mut field,
            &context,
            &mut out,
        );
        let s = &mut field.get_player_mut(GIVER).unwrap().statistics;
        assert_eq!(s.miscontrols + s.heavy_touches, 1, "tick {minute_ms}");
        s.miscontrols = 0;
        s.heavy_touches = 0;
    }
}

/// Whether `GIVER` defends the left-hand goal.
fn defends_left(field: &MatchField) -> bool {
    field.get_player(GIVER).and_then(|p| p.side) == Some(PlayerSide::Left)
}

/// The home clearer on the ball just outside his own box, striking it at
/// `velocity`, miscued or not. Returns the field and the ball's velocity.
fn clear_from_the_box(miscued: bool, velocity: Vector3<f32>) -> (MatchField, Vector3<f32>) {
    let (mut field, mut context) = kickoff();
    let at = Vector3::new(if defends_left(&field) { 140.0 } else { 700.0 }, 272.0, 0.0);
    if let Some(p) = field.get_player_mut(GIVER) {
        p.position = at;
    }
    field.ball.position = Vector3::new(at.x, at.y, 0.2);
    field.ball.velocity = Vector3::zeros();
    field.ball.current_owner = Some(GIVER);
    let event = if miscued {
        PlayerEvent::MiscuedClearance(GIVER, velocity)
    } else {
        PlayerEvent::ClearBall(GIVER, velocity)
    };
    let mut data = ResultMatchPositionData::new();
    PlayerEventDispatcher::dispatch(event, &mut field, &mut context, &mut data);
    let after = field.ball.velocity;
    (field, after)
}

#[test]
fn a_miscued_clearance_is_his_giveaway() {
    let (miscued, _) = clear_from_the_box(true, Vector3::new(1.5, 0.4, 0.3));
    assert_eq!(miscued.ball.last_giveaway_player_id, Some(GIVER));
    let (clean, _) = clear_from_the_box(false, Vector3::new(1.5, 0.4, 0.3));
    assert_eq!(clean.ball.last_giveaway_player_id, None);
}

#[test]
fn only_a_miscue_may_go_toward_his_own_goal() {
    let (field, _) = kickoff();
    let left = defends_left(&field);
    let toward_goal = Vector3::new(if left { -1.2 } else { 1.2 }, 0.3, 0.3);
    let backward = |v: Vector3<f32>| if left { v.x < 0.0 } else { v.x > 0.0 };
    let (_, clean) = clear_from_the_box(false, toward_goal);
    assert!(!backward(clean), "an ordinary clearance is turned away from goal");
    let (_, miscued) = clear_from_the_box(true, toward_goal);
    assert!(backward(miscued), "a miscue goes where it was hit");
}

/// A median clearer, nerves below the point where the lane's ceiling
/// binds: his miscues rise with them.
#[test]
fn a_nervous_clearer_miscues_more() {
    let (mut field, _) = kickoff();
    if let Some(p) = field.get_player_mut(GIVER) {
        p.skills.mental.concentration = 10.0;
        p.skills.mental.composure = 10.0;
        p.skills.mental.decisions = 10.0;
        p.attributes.consistency = 10.0;
    }
    let clearer = field.get_player(GIVER).unwrap();
    let settled = ActionLapse::probability(LapseFamily::Clearance, clearer, Some(&PsychState::default()));
    let nervous = ActionLapse::probability(
        LapseFamily::Clearance,
        clearer,
        Some(&PsychState {
            nervousness: 0.15,
            ..PsychState::default()
        }),
    );
    assert!(nervous > settled * 1.4, "{nervous} against {settled}");
}
